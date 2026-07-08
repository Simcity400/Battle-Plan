use crate::compact::{
    decode_payload, encode_payload, summarize_snapshot, AgencyCapacityClusterRow, AgencyRow,
    CountryDiplomacyRow, CryptologyRow, DiplomacyIntelSnapshot, Domain53GateSummary, FactionRow,
    IncomingActionQueueRow, IntelMapRow, IntelNetworkGraphRow, OperationAnchorRow,
    OperationInstanceRow, OperativeRow, OpinionRow, RelationFlagRow, StateEventRow, SubjectRow,
    WarRelationRow, DOMAIN_ID_PROPOSED, MAX_ACTION_DATE_HOURS, MAX_AGENCY_CAPACITY_CLUSTER_VALUE,
    MAX_CASUALTIES, MAX_CRYPTO_AMOUNT_RAW, MAX_FIXED_RAW, MAX_INTEL_NETWORK_STATE_ID,
    MAX_OPERATION_ID, MAX_OPERATIVE_ID, MAX_OPERATIVE_LIFECYCLE_FLAG, MAX_OPERATIVE_SKILL,
    MAX_OPINION_ABS, MAX_TAG_DB_ID, MAX_XP_RAW, REL_KIND_AIR_BASE_ACCESS, REL_KIND_ATTACHE,
    REL_KIND_DOCKING_RIGHTS, REL_KIND_EMBARGO, REL_KIND_EQUIPMENT_PURCHASE_CONTRACT_RELATION,
    REL_KIND_GUARANTEE, REL_KIND_IMPROVE_RELATIONS, REL_KIND_LICENCE_PRODUCTION,
    REL_KIND_MARKET_ACCESS_RIGHTS, REL_KIND_MILITARY_ACCESS, REL_KIND_NAVAL_BLOCKADE,
    REL_KIND_NON_AGGRESSION_PACT, REL_KIND_PUPPET, STATE_EVENT_KIND_OBSERVED,
    STATE_EVENT_SOURCE_AGENCY_CAPACITY, STATE_EVENT_SOURCE_AGENCY_SCALAR,
    STATE_EVENT_SOURCE_COUNTRY_FACTION_GROUP, STATE_EVENT_SOURCE_CRYPTOLOGY,
    STATE_EVENT_SOURCE_OPERATIVE_LIFECYCLE, STATE_EVENT_SOURCE_RELATION_FLAG,
    STATE_EVENT_SOURCE_RELATION_OPINION, STATE_EVENT_SOURCE_SUBJECT_AUTONOMY,
    STATUS_FACTION_GROUP_UNVALIDATED, STATUS_INCOMING_ACTION_QUEUE_UNVALIDATED,
    STATUS_INTEL_MAP_RESISTED, STATUS_INTEL_NETWORK_GRAPH_UNVALIDATED,
    STATUS_OPERATIONS_DETAIL_RESISTED, STATUS_WAR_LIFECYCLE_DETAIL_UNVALIDATED,
    STATUS_WAR_ROOT_UNVALIDATED,
};
use compact_contract::build_profile as profile;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

pub const EXPECTED_HOI4_SHA256_870EBF: &str =
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";

pub const SOURCE_PROOF_BUILD_SHA_870EBF: u64 = 1 << 0;
pub const SOURCE_PROOF_COUNTRY_ROOTS_870EBF: u64 = 1 << 1;
pub const SOURCE_PROOF_RELATION_ROOTS_870EBF: u64 = 1 << 2;
pub const SOURCE_PROOF_AGENCY_ROOT_870EBF: u64 = 1 << 3;
pub const SOURCE_PROOF_CRYPTOLOGY_ROOT_870EBF: u64 = 1 << 4;
pub const SOURCE_PROOF_OPERATIVE_ROOT_870EBF: u64 = 1 << 5;
pub const SOURCE_PROOF_WAR_ROOT_870EBF: u64 = 1 << 6;
pub const SOURCE_PROOF_OPERATION_DETAIL_870EBF: u64 = 1 << 7;
pub const SOURCE_PROOF_INTEL_MAP_PERCENTAGES_870EBF: u64 = 1 << 8;
pub const SOURCE_PROOF_INCOMING_ACTION_QUEUE_870EBF: u64 = 1 << 9;
pub const SOURCE_PROOF_WAR_LIFECYCLE_DETAIL_870EBF: u64 = 1 << 10;
pub const SOURCE_PROOF_FACTION_GROUP_870EBF: u64 = 1 << 11;
pub const SOURCE_PROOF_INTEL_NETWORK_GRAPH_870EBF: u64 = 1 << 12;
/// CFactionSystem roster (`CGameState+0x3F8`) — faction id/type/name/members/
/// leader. Distinct from SOURCE_PROOF_FACTION_GROUP (per-country ideology bucket).
pub const SOURCE_PROOF_FACTION_SYSTEM_870EBF: u64 = 1 << 13;

pub const KNOWN_SOURCE_PROOFS: u64 = SOURCE_PROOF_BUILD_SHA_870EBF
    | SOURCE_PROOF_COUNTRY_ROOTS_870EBF
    | SOURCE_PROOF_RELATION_ROOTS_870EBF
    | SOURCE_PROOF_AGENCY_ROOT_870EBF
    | SOURCE_PROOF_CRYPTOLOGY_ROOT_870EBF
    | SOURCE_PROOF_OPERATIVE_ROOT_870EBF
    | SOURCE_PROOF_WAR_ROOT_870EBF
    | SOURCE_PROOF_OPERATION_DETAIL_870EBF
    | SOURCE_PROOF_INTEL_MAP_PERCENTAGES_870EBF
    | SOURCE_PROOF_INCOMING_ACTION_QUEUE_870EBF
    | SOURCE_PROOF_WAR_LIFECYCLE_DETAIL_870EBF
    | SOURCE_PROOF_FACTION_GROUP_870EBF
    | SOURCE_PROOF_INTEL_NETWORK_GRAPH_870EBF
    | SOURCE_PROOF_FACTION_SYSTEM_870EBF;

pub const CURRENT_VALIDATED_SOURCE_PROOFS: u64 = SOURCE_PROOF_BUILD_SHA_870EBF
    | SOURCE_PROOF_COUNTRY_ROOTS_870EBF
    | SOURCE_PROOF_RELATION_ROOTS_870EBF
    | SOURCE_PROOF_AGENCY_ROOT_870EBF
    | SOURCE_PROOF_CRYPTOLOGY_ROOT_870EBF
    | SOURCE_PROOF_OPERATIVE_ROOT_870EBF
    | SOURCE_PROOF_WAR_ROOT_870EBF
    | SOURCE_PROOF_INCOMING_ACTION_QUEUE_870EBF
    | SOURCE_PROOF_FACTION_GROUP_870EBF
    | SOURCE_PROOF_INTEL_NETWORK_GRAPH_870EBF
    | SOURCE_PROOF_FACTION_SYSTEM_870EBF;

const RTTI_CCOUNTRY: &str = "CCountry";
const RTTI_CINTELLIGENCE_AGENCY: &str = "CIntelligenceAgency";
const DOMAIN53_RTTI_MAX_NAME_LEN: usize = 200;
const DOMAIN53_MODULE_SPAN_MAX: u64 = 0x5000_0000;
const DOMAIN53_FACTION_PROBE_COUNTRY_FLOOR: usize = 40;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CurrentBuildRoot {
    pub name: &'static str,
    pub evidence: &'static str,
}

pub const CURRENT_BUILD_ROOTS_870EBF: &[CurrentBuildRoot] = &[
    CurrentBuildRoot {
        name: "game_state",
        evidence: "u64[module+0x3316E90]",
    },
    CurrentBuildRoot {
        name: "country_database",
        evidence: "u64[module+0x33189C8]",
    },
    CurrentBuildRoot {
        name: "country_table",
        evidence: "game_state+0x310 table, game_state+0x31C count, country vtable module+0x27D0E90",
    },
    CurrentBuildRoot {
        name: "relations",
        evidence: "country+0xF88 -> manager, manager+0x08 -> table, relation+0x300 opinion",
    },
    CurrentBuildRoot {
        name: "subject_relation",
        evidence: "CSubjectRelation module+0x2949F18, overlord +0x10, subject +0x14, autonomy +0x58/+0x60",
    },
    CurrentBuildRoot {
        name: "agency",
        evidence: "country+0xFC0 -> CIntelligenceAgency, vtable module+0x296B338, capacity raw cluster agency+0xE4/+0xF0/+0xF4 with +0xE4==+0xF4 warm invariant",
    },
    CurrentBuildRoot {
        name: "cryptology",
        evidence: "agency+0x120 -> CCryptology module+0x29A6540; records manager+0x28 stride 0x38, state vtable module+0x29A64F0",
    },
    CurrentBuildRoot {
        name: "operative_identity",
        evidence: "COperativeLeader vtable/classdesc module+0x293F790, identity+0x110 lifecycle flag cohorts {47,31,63}, owner identity+0x118, skill/XP stats path",
    },
    CurrentBuildRoot {
        name: "war_relation_rows",
        evidence: "global row pool with tag ids +0x00/+0x04, date vtable +0x08 in {CGregorianDate module+0x2702190, CGameDate module+0x2702388}, casualties +0x40/+0x48 i32, echo +0x68/+0x6C; v13 warm validation 850/850",
    },
    CurrentBuildRoot {
        name: "incoming_diplomatic_action_queue",
        evidence: "country+0xFF8 -> queue, queue+0x08 action pointer array, queue+0x10 size/cap, queue+0x20 recipient guard, CIncomingDiplomaticAction vtable module+0x2A209A0, handle +0x08, body +0x18 actor/recipient/date fields",
    },
    CurrentBuildRoot {
        name: "intel_network_graph",
        evidence: "CStrategicOperative vtable module+0x298B448, +0x08 operative index, +0x10 vector<CCountryIntelNetwork*>, +0x18 capacity, +0x1C size; CCountryIntelNetwork vtable module+0x298AB48, target +0x08, graph +0x18, graph+0x38 vertex array, graph+0x40 vertex count, 16-byte vertices state_id@+0",
    },
];

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExtractedDomain53Rows {
    pub elapsed_wall_ms: u64,
    pub changed_rows: Option<u32>,
    pub countries: Vec<CountryDiplomacyRow>,
    pub wars: Vec<WarRelationRow>,
    pub opinions: Vec<OpinionRow>,
    pub relation_flags: Vec<RelationFlagRow>,
    pub subjects: Vec<SubjectRow>,
    pub agencies: Vec<AgencyRow>,
    pub agency_capacity_clusters: Vec<AgencyCapacityClusterRow>,
    pub operatives: Vec<OperativeRow>,
    pub cryptology: Vec<CryptologyRow>,
    pub operation_anchors: Vec<OperationAnchorRow>,
    pub intel_map: Vec<IntelMapRow>,
    pub state_events: Vec<StateEventRow>,
    /// Wave-5 running-operation instances (heap-discovered COperationInstance).
    pub operation_instances: Vec<OperationInstanceRow>,
    pub incoming_actions: Vec<IncomingActionQueueRow>,
    pub intel_network_graphs: Vec<IntelNetworkGraphRow>,
    /// CFactionSystem roster (carried in the FACS trailer).
    pub factions: Vec<FactionRow>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExtractError {
    MemoryRead { address: u64, len: usize },
    InvalidRoot(&'static str),
    InvalidCount { name: &'static str, value: u32 },
    Validation(String),
}

pub trait Domain53Memory {
    fn module_base(&self) -> u64;
    fn read_bytes(&self, address: u64, len: usize) -> Result<Vec<u8>, ExtractError>;

    /// Allocation-free sibling of [`read_bytes`](Self::read_bytes): fill a
    /// caller-owned `buf` from `[address, address + buf.len())`. The DEFAULT impl
    /// delegates to `self.read_bytes(address, buf.len())` and copies the result
    /// into `buf`, so every existing implementor (fixtures, test shims, replay
    /// mocks) keeps its exact accept/reject behavior with no code change — the
    /// default is byte-identical to the caller doing `read_bytes` itself.
    ///
    /// Collector-side impls MAY override this with an alloc-free primitive, but
    /// ONLY when that primitive's accept/reject decision is provably identical to
    /// their own `read_bytes` (same gate, same all-or-nothing copy).
    ///
    /// On `Err`, `buf`'s contents are unspecified. Callers that fail-close on the
    /// error (e.g. `skip()`+`continue` without scanning `buf`) are safe to reuse
    /// the buffer across calls; a caller that inspects `buf` after an `Err` MUST
    /// zero it itself.
    fn read_into(&self, address: u64, buf: &mut [u8]) -> Result<(), ExtractError> {
        let bytes = self.read_bytes(address, buf.len())?;
        buf.copy_from_slice(&bytes);
        Ok(())
    }

    fn readable_ranges(&self, _lo: u64, _hi: u64) -> Vec<(u64, usize)> {
        Vec::new()
    }

    fn read_u8(&self, address: u64) -> Result<u8, ExtractError> {
        let bytes = self.read_bytes(address, 1)?;
        Ok(bytes[0])
    }

    fn read_u32(&self, address: u64) -> Result<u32, ExtractError> {
        let bytes = self.read_bytes(address, 4)?;
        Ok(u32::from_le_bytes(
            bytes.try_into().expect("u32 byte count"),
        ))
    }

    fn read_i32(&self, address: u64) -> Result<i32, ExtractError> {
        let bytes = self.read_bytes(address, 4)?;
        Ok(i32::from_le_bytes(
            bytes.try_into().expect("i32 byte count"),
        ))
    }

    fn read_u64(&self, address: u64) -> Result<u64, ExtractError> {
        let bytes = self.read_bytes(address, 8)?;
        Ok(u64::from_le_bytes(
            bytes.try_into().expect("u64 byte count"),
        ))
    }

    fn read_i64(&self, address: u64) -> Result<i64, ExtractError> {
        let bytes = self.read_bytes(address, 8)?;
        Ok(i64::from_le_bytes(
            bytes.try_into().expect("i64 byte count"),
        ))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Domain53FamilyCounts {
    pub country_roots: u32,
    pub relation_rows: u32,
    pub agency_roots: u32,
    pub operative_scan_ranges: u32,
    pub war_scan_ranges: u32,
    /// Operation-instance observability (A+C fix): how the candidate hits split
    /// this frame, so operation completeness is visible on a SUCCESSFUL frame and
    /// a real operation-discovery regression can be seen without the family
    /// nuking the whole domain. `candidates = emitted + stale_liveness +
    /// filtered`.
    pub operation_candidates: u32,
    pub operation_emitted: u32,
    pub operation_stale_liveness: u32,
    pub operation_filtered: u32,
}

/// W1 instrument (perf_rearch_20260706 T4): per-family wall time (elapsed
/// micros) accumulated across the family call sites in
/// [`extract_rows_from_resolved_roots_with_discovery`]. Observational ONLY —
/// populated by wrapping each family call in `Instant::now()`/`.elapsed()`;
/// it never influences rows, wire bytes, status flags, error variants, or
/// control flow. The collector plumbs these into the d53 DOMAIN_PERF diagnostic
/// trailer (`domain53_phase_*_us` units, the same telemetry channel as the
/// existing `domain53_*` diag units — the manifest aggregates them by name into
/// `data_units_total`) so a live capture can attribute d53 frame time per family.
/// These units ride the DOMAIN_PERF (telemetry) record, never the folded d53
/// DATA snapshot. On early-`Err` returns the frame (and this struct) never
/// escapes, so partial phase times are simply not reported — intentional.
///
/// W1.1 extension (perf_rearch_20260706): three additional lane-side segment
/// timers that attribute the d53 wall time NOT already under one of the 10
/// family timers, so `roots_us + <10 families> + war_projection_us +
/// aggregation_us` accounts for ~all of the lane extract wall (the collector
/// measures that extract wall as `seg_extract_us`, letting the analysis confirm
/// the lane-internal residual is small). The three new fields are:
/// - `roots_us`: [`resolve_domain53_frame_roots`] (country-table walk/validate),
///   which the wrapper entry points run BEFORE this function — carried in via
///   `Domain53FrameRoots::roots_resolve_us`.
/// - `war_projection_us`: the war-degraded decision + `war_projection_by_country`
///   fold, an untimed span BETWEEN the `wars_us` and `faction_gate_us` timers.
/// - `aggregation_us`: the two final per-agency aggregation loops
///   (operative_count then cryptology_target_count), untimed spans AROUND the
///   `cryptology_us` timer. These new fields never overlap the existing 10
///   (they cover disjoint code spans) so the sum stays a partition of the
///   extract wall — no double-counting.
///
/// All 13 fields are observational: they change no row, wire byte, status flag,
/// error variant, or control flow, and (like the original 10) are simply not
/// reported on an early-`Err` return.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Domain53PhaseTimes {
    pub wars_us: u64,
    pub faction_gate_us: u64,
    pub relations_us: u64,
    pub faction_roster_us: u64,
    pub incoming_actions_us: u64,
    pub agencies_us: u64,
    pub operatives_us: u64,
    pub operations_us: u64,
    pub intel_networks_us: u64,
    pub cryptology_us: u64,
    /// W1.1: country-root resolution wall (see struct doc). Carried from
    /// `Domain53FrameRoots::roots_resolve_us`.
    pub roots_us: u64,
    /// W1.1: war-degraded decision + `war_projection_by_country` fold wall
    /// (between the wars and faction_gate family timers).
    pub war_projection_us: u64,
    /// W1.1: final per-agency aggregation loops (operative_count +
    /// cryptology_target_count) wall.
    pub aggregation_us: u64,
}

/// Elapsed micros as `u64`, saturating (a d53 frame is milliseconds, never
/// ~584_000 years, so the clamp is a formality that keeps the timer branchless
/// and panic-free). Used only by the W1 per-family timers; observational.
fn duration_micros_u64_saturating(elapsed: std::time::Duration) -> u64 {
    elapsed.as_micros().min(u128::from(u64::MAX)) as u64
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtractedDomain53Frame {
    pub rows: ExtractedDomain53Rows,
    pub source_proofs: u64,
    pub observed_rows: u32,
    pub skipped_rows: u32,
    pub stale_dropped_total: usize,
    pub family_counts: Domain53FamilyCounts,
    /// W1 per-family wall timers (perf_rearch_20260706 T4). Observational only;
    /// see [`Domain53PhaseTimes`].
    pub phase_times: Domain53PhaseTimes,
    /// W3-a0 relation-flag match-offset histogram (perf_rearch_20260706).
    /// Observational only; see [`RelationFlagOffsetHistogram`]. Deterministic
    /// given the scanned blobs, so it does not perturb the derived `PartialEq`.
    pub relflag_offsets: RelationFlagOffsetHistogram,
}

pub const DOMAIN53_DISCOVERY_DEGRADED: &str = "domain53_discovery_degraded";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Domain53Discovery {
    pub operative_hits: Vec<u64>,
    pub war_hits: [Vec<u64>; 2],
    /// Wave-5 running-operations: COperationInstance vtable hits from the same
    /// background heap pass (no extra thread — rides the existing discovery
    /// cache). Consumed and revalidated fresh each frame.
    pub operation_instance_hits: Vec<u64>,
    pub strategic_operative_hits: Vec<u64>,
    pub country_intel_network_hits: Vec<u64>,
    pub scan_range_count: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CountryRoot {
    tag_db_id: u32,
    country_ptr: u64,
}

/// d53 relation-flag CONTAINER diagnostic (perf_rearch_20260706, RELATION_CONTAINER_RE).
/// Per-`flag_kind` COUNT of relation sub-objects found in each pair's
/// `pdx::vector<CRelation*>` (`rel+0xD0`/count `rel+0xDC`), accumulated across the
/// whole frame's relation walk. Records EVERY known-kind container entry — including
/// the kinds d53 does NOT emit here (PUPPET → +0x288; MARKET_ACCESS_RIGHTS → d43;
/// AIR_BASE_ACCESS → d30) — so the live per-kind totals can be cross-checked against
/// the melt oracle (e.g. NAP 615, docking_rights 165, military_access 43 on D04 1944).
///
/// HISTORY: this struct previously held the inline-scan match-OFFSET bitmask (the
/// W3-a0 instrument that proved the inline scan matched 0). The inline scan is now
/// removed; the field `per_kind[K].0` is repurposed from an offset bitmask to a plain
/// COUNT, and `.1` is left 0. Surfaced UNCHANGED through the collector's DOMAIN_PERF
/// trailer as `domain53_relflag_k{K}_offlo` (= per-kind count) and
/// `domain53_relflag_total_matches` (= total known-kind container entries). No collector
/// change. Purely diagnostic: it never changes a row, wire byte, guarded read, status,
/// error, or control flow, and is deterministic given the walked containers (so it does
/// not perturb the derived `PartialEq` of the frame).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RelationFlagOffsetHistogram {
    /// Per-`flag_kind` container-entry COUNT in `.0` (`.1` unused, left 0). A kind
    /// absent from the map had zero container entries this frame.
    pub per_kind: BTreeMap<u16, (u64, u64)>,
    /// Total known-kind relation sub-objects walked this frame (== sum of `per_kind[*].0`).
    pub total_matches: u64,
}

impl RelationFlagOffsetHistogram {
    /// Count one relation-flag container sub-object of `flag_kind`. Bumps the kind's
    /// `.0` count and `total_matches` (saturating; container entries are few per pair).
    /// Diagnostic only.
    fn record_kind_count(&mut self, flag_kind: u16) {
        let entry = self.per_kind.entry(flag_kind).or_default();
        entry.0 = entry.0.saturating_add(1);
        self.total_matches = self.total_matches.saturating_add(1);
    }
}

#[derive(Clone, Debug, Default)]
struct RowAccumulator {
    rows: ExtractedDomain53Rows,
    skipped_rows: u32,
    observed_rows: u32,
    /// W3-a0 observational instrument: relation-flag match-offset histogram,
    /// accumulated at the relation scan's match site. Diagnostic only (see
    /// [`RelationFlagOffsetHistogram`]).
    relflag_offsets: RelationFlagOffsetHistogram,
}

impl RowAccumulator {
    fn skip(&mut self) {
        self.skipped_rows = self.skipped_rows.saturating_add(1);
    }

    fn observe(&mut self) {
        self.observed_rows = self.observed_rows.saturating_add(1);
    }
}

#[derive(Clone, Debug)]
struct AgencyRootRead {
    owner_tag_db_id: u32,
    agency_ptr: u64,
}

#[derive(Clone, Debug)]
struct Domain53FrameRoots {
    module_base: u64,
    game_state: u64,
    current_game_hour: u32,
    countries: Vec<CountryRoot>,
    /// W1.1 instrument (perf_rearch_20260706): wall micros spent inside
    /// [`resolve_domain53_frame_roots`] — global-root reads + build-identity
    /// verify + the full country-table walk/validation
    /// ([`resolve_country_roots`]). Measured at the SINGLE resolve site so all
    /// three `extract_rows_from_current_roots*` entry points inherit it, then
    /// copied verbatim into `Domain53PhaseTimes::roots_us` at the top of
    /// [`extract_rows_from_resolved_roots_with_discovery`]. Observational only:
    /// it is never read by any control-flow / row / status decision; it exists
    /// solely to carry the timing across the resolve→extract boundary.
    roots_resolve_us: u64,
}

#[derive(Clone, Debug, Default)]
struct Domain53DiscoveryStaleness {
    operative_total: usize,
    operative_stale_dropped: usize,
    war_total: usize,
    war_stale_dropped: usize,
    // Wave-5 running operations: SEPARATELY attributed. Operations churn on a
    // different cadence than operatives and — crucially — the operation family
    // does NOT gate the whole d53 frame (A+C fix, see `frame_degraded`): only
    // GENUINE liveness failure is counted here, and even a majority of it merely
    // means fewer operation rows, never the loss of operatives/relations/etc.
    // Kept distinct from `operative_*` so neither family's noise touches the
    // other's accounting.
    operation_total: usize,
    /// GENUINE operation liveness failures only (vtable mismatch / unreadable
    /// handle). Does NOT include candidate-rejection filtering — see
    /// [`OperationDropBreakdown`] and [`Self::operation_degraded`].
    operation_stale_dropped: usize,
    /// Operation candidates rejected as not-a-unique-live-instance (bad
    /// handle_type / id, duplicate). Diagnostic only; never gates.
    operation_filtered: usize,
}

impl Domain53DiscoveryStaleness {
    /// WHOLE-FRAME fail-closed gate. A stale-majority of the OPERATIVE family
    /// degrades the entire d53 frame to `SOURCE_UNAVAILABLE`: operatives have no
    /// first-class per-family "emit-anyway-unvalidated" wire status, so the only
    /// honest fail-closed outcome for a systemically stale operative set is to
    /// withhold the whole frame. Because every family is reassembled from ONE
    /// shared background sweep generation, a genuinely dead/ancient sweep makes
    /// operatives stale too, so the operative gate is the reliable systemic-death
    /// signal for the whole domain.
    ///
    /// OPERATIONS are deliberately NOT in this gate (A+C fix, 2026-07-07; see
    /// runs/perf_rearch_20260706/D53_ROOT_CAUSE.md). Live speed-5 showed the
    /// operation family's "stale majority" was dominated by discovery
    /// FALSE-POSITIVES / de-duplication (bad handle_type, duplicate id), NOT
    /// object death — genuine operation liveness failure was only ~21%. Operation
    /// rows are already row-local fail-closed (each emitted row is individually
    /// re-validated: vtable + handle_type + id + duplicate guard), exactly like
    /// operative row-decode failures (which `skip`, never nuke). So a high
    /// operation drop-rate must only mean fewer operation rows that frame — it
    /// must NOT erase the unrelated d53 families (relations, agencies,
    /// cryptology, factions, intel graphs, wars). Operation health is surfaced
    /// via [`Self::operation_degraded`] as a diagnostic, not a frame nuke.
    ///
    /// The WAR family is likewise NOT in this gate: it has an explicit per-family
    /// wire status (`STATUS_WAR_ROOT_UNVALIDATED` + absent
    /// `SOURCE_PROOF_WAR_ROOT_870EBF`) and DOWNGRADES via [`Self::war_degraded`].
    fn frame_degraded(&self) -> bool {
        stale_fraction_degraded(self.operative_total, self.operative_stale_dropped)
    }

    /// DIAGNOSTIC ONLY — never nukes the frame. True when GENUINE operation
    /// liveness failures (vtable mismatch / unreadable handle, NOT filtering) are
    /// a majority of the candidate set: a signal that the operation discovery has
    /// genuinely decayed. Operation health is surfaced in production via
    /// `family_counts` (candidates/emitted/stale_liveness/filtered); this method
    /// states the "majority genuine death" predicate for the contract tests and
    /// is kept as the documented definition it references from
    /// [`Self::frame_degraded`].
    #[allow(dead_code)]
    fn operation_degraded(&self) -> bool {
        stale_fraction_degraded(self.operation_total, self.operation_stale_dropped)
    }

    /// WAR-FAMILY-ONLY downgrade gate. A stale-majority of the small validated
    /// war-address set means the cached war set is systemically stale (e.g. the
    /// sweep generation is old relative to speed-5 war churn). Rather than nuke
    /// the whole frame, the extractor emits with the war family marked
    /// unvalidated; the next completed sweep generation rebuilds + revalidates
    /// the war set off-frame.
    fn war_degraded(&self) -> bool {
        stale_fraction_degraded(self.war_total, self.war_stale_dropped)
    }

    fn stale_dropped_total(&self) -> usize {
        self.operative_stale_dropped
            .saturating_add(self.war_stale_dropped)
            .saturating_add(self.operation_stale_dropped)
    }
}

fn stale_fraction_degraded(total: usize, stale: usize) -> bool {
    total >= 8 && stale.saturating_mul(2) > total
}

pub fn discover_domain53_heap_hits<M: Domain53Memory>(
    mem: &M,
) -> Result<Domain53Discovery, ExtractError> {
    let module_base = mem.module_base();
    if module_base == 0 {
        return Err(ExtractError::InvalidRoot("module_base_zero"));
    }
    verify_domain53_build_identity(mem, module_base)?;
    discover_domain53_heap_hits_for_module(mem, module_base)
}

/// P3 shared-sweep migration: the six absolute vtable needles the domain-53
/// heap discovery matches, in the order
/// `[operative, war_gregorian_date, war_game_date, operation_instance,
/// strategic_operative, country_intel_network]`. The collector registers these
/// with the shared background `heap_sweep` service and reassembles a
/// [`Domain53Discovery`] from the published per-needle hit vectors via
/// [`domain53_discovery_from_needle_hits`] — no inline heap scan on the frame
/// thread. Build identity is still verified per frame during row extraction, so
/// a wrong module base only yields empty/garbage hits that fail closed.
pub fn domain53_scan_needles(module_base: u64) -> Result<[u64; 6], ExtractError> {
    if module_base == 0 {
        return Err(ExtractError::InvalidRoot("module_base_zero"));
    }
    let date_vtables = domain53_war_date_vtables(module_base)?;
    Ok([
        add_addr(module_base, profile::DOMAIN53_OPERATIVE_VTABLE_RVA)?,
        date_vtables[0],
        date_vtables[1],
        add_addr(module_base, profile::DOMAIN53_OPERATION_INSTANCE_VTABLE_RVA)?,
        add_addr(
            module_base,
            profile::DOMAIN53_STRATEGIC_OPERATIVE_VTABLE_RVA,
        )?,
        add_addr(
            module_base,
            profile::DOMAIN53_COUNTRY_INTEL_NETWORK_VTABLE_RVA,
        )?,
    ])
}

/// Reassemble a [`Domain53Discovery`] from hits produced by the shared sweep
/// service for the needles returned by [`domain53_scan_needles`] (same order).
/// Byte-for-byte equivalent to what `discover_domain53_heap_hits_for_module`
/// would have produced from its own scan; every hit is re-validated per frame
/// during extraction (ABA-safe).
pub fn domain53_discovery_from_needle_hits(
    operative_hits: Vec<u64>,
    war_gregorian_hits: Vec<u64>,
    war_game_date_hits: Vec<u64>,
    operation_instance_hits: Vec<u64>,
    strategic_operative_hits: Vec<u64>,
    country_intel_network_hits: Vec<u64>,
) -> Domain53Discovery {
    Domain53Discovery {
        operative_hits,
        war_hits: [war_gregorian_hits, war_game_date_hits],
        operation_instance_hits,
        strategic_operative_hits,
        country_intel_network_hits,
        scan_range_count: 0,
    }
}

fn discover_domain53_heap_hits_for_module<M: Domain53Memory>(
    mem: &M,
    module_base: u64,
) -> Result<Domain53Discovery, ExtractError> {
    let ranges = mem.readable_ranges(
        profile::DOMAIN53_HEAP_LO as u64,
        profile::DOMAIN53_HEAP_HI as u64,
    );
    let operative_vtable = add_addr(module_base, profile::DOMAIN53_OPERATIVE_VTABLE_RVA)?;
    let date_vtables = domain53_war_date_vtables(module_base)?;
    let operation_instance_vtable =
        add_addr(module_base, profile::DOMAIN53_OPERATION_INSTANCE_VTABLE_RVA)?;
    let strategic_operative_vtable = add_addr(
        module_base,
        profile::DOMAIN53_STRATEGIC_OPERATIVE_VTABLE_RVA,
    )?;
    let country_intel_network_vtable = add_addr(
        module_base,
        profile::DOMAIN53_COUNTRY_INTEL_NETWORK_VTABLE_RVA,
    )?;
    Ok(Domain53Discovery {
        operative_hits: scan_qword_hits(mem, &ranges, operative_vtable)?,
        war_hits: [
            scan_qword_hits(mem, &ranges, date_vtables[0])?,
            scan_qword_hits(mem, &ranges, date_vtables[1])?,
        ],
        // Wave-5: same heap pass, one more needle (the COperationInstance
        // vtable). Consumed by read_operations, revalidated fresh per frame.
        operation_instance_hits: scan_qword_hits(mem, &ranges, operation_instance_vtable)?,
        strategic_operative_hits: scan_qword_hits(mem, &ranges, strategic_operative_vtable)?,
        country_intel_network_hits: scan_qword_hits(mem, &ranges, country_intel_network_vtable)?,
        scan_range_count: ranges.len().min(u32::MAX as usize) as u32,
    })
}

pub fn extract_rows_from_current_roots<M: Domain53Memory>(
    mem: &M,
) -> Result<ExtractedDomain53Frame, ExtractError> {
    let roots = resolve_domain53_frame_roots(mem)?;
    let discovery = discover_domain53_heap_hits_for_module(mem, roots.module_base)?;
    extract_rows_from_resolved_roots_with_discovery(mem, roots, &discovery, None)
}

pub fn extract_rows_from_current_roots_with_discovery<M: Domain53Memory>(
    mem: &M,
    discovery: &Domain53Discovery,
) -> Result<ExtractedDomain53Frame, ExtractError> {
    let roots = resolve_domain53_frame_roots(mem)?;
    extract_rows_from_resolved_roots_with_discovery(mem, roots, discovery, None)
}

/// P4b entry point: the war rows are re-read from a SMALL set of validated
/// addresses (produced off-frame by [`domain53_validate_war_row_addrs`]) instead
/// of the raw ~1.47M war-date hits. `discovery.war_hits` is ignored — the
/// operative/operation/spy-net families still come from `discovery`; only the
/// war family switches to the validated-address source. War-row VALUES (tags,
/// casualties) are re-read live from those addresses every frame.
pub fn extract_rows_from_current_roots_with_validated_war_addrs<M: Domain53Memory>(
    mem: &M,
    discovery: &Domain53Discovery,
    war_row_addrs: &[u64],
) -> Result<ExtractedDomain53Frame, ExtractError> {
    let roots = resolve_domain53_frame_roots(mem)?;
    extract_rows_from_resolved_roots_with_discovery(mem, roots, discovery, Some(war_row_addrs))
}

fn resolve_domain53_frame_roots<M: Domain53Memory>(
    mem: &M,
) -> Result<Domain53FrameRoots, ExtractError> {
    // W1.1 instrument: time the whole resolve span (global-root reads +
    // build-identity verify + country-table walk/validation). Observational —
    // the timer is stashed on the returned struct and never influences control
    // flow; on any early `?` return the frame never escapes, so no partial time
    // is reported (identical to the family-timer contract). Every existing `?`
    // below is unchanged.
    let roots_started = Instant::now();
    let module_base = mem.module_base();
    if module_base == 0 {
        return Err(ExtractError::InvalidRoot("module_base_zero"));
    }

    let game_state = mem.read_u64(add_addr(module_base, profile::CG_STATE_DATE_OFFSET)?)?;
    let country_database =
        mem.read_u64(add_addr(module_base, profile::COUNTRY_DATABASE_OFFSET)?)?;
    if !is_heap_ptr(game_state) || !is_heap_ptr(country_database) {
        return Err(ExtractError::InvalidRoot(
            "domain53_global_roots_unavailable",
        ));
    }
    verify_domain53_build_identity(mem, module_base)?;

    let countries = resolve_country_roots(mem, module_base, game_state)?;
    if countries.is_empty() {
        return Err(ExtractError::InvalidRoot(
            "domain53_country_roots_empty_implausible",
        ));
    }

    let current_game_hour =
        mem.read_u32(add_addr(game_state, profile::CGAMESTATE_CURRENT_HOUR)?)?;

    Ok(Domain53FrameRoots {
        module_base,
        game_state,
        current_game_hour,
        countries,
        roots_resolve_us: duration_micros_u64_saturating(roots_started.elapsed()),
    })
}

fn extract_rows_from_resolved_roots_with_discovery<M: Domain53Memory>(
    mem: &M,
    roots: Domain53FrameRoots,
    discovery: &Domain53Discovery,
    // P4b: when `Some`, the war family is re-read from this small validated
    // address set (produced off-frame) instead of `discovery.war_hits` (the raw
    // ~1.47M hits). `None` keeps the original inline path (used by direct
    // callers and tests). The operative/operation/spy-net families always come
    // from `discovery`.
    war_row_addrs: Option<&[u64]>,
) -> Result<ExtractedDomain53Frame, ExtractError> {
    let module_base = roots.module_base;
    let game_state = roots.game_state;
    let current_game_hour = roots.current_game_hour;
    // W1.1: carry the root-resolution wall (measured off in the resolve site
    // BEFORE this call) into the phase accumulator. Observational only.
    let roots_resolve_us = roots.roots_resolve_us;
    let countries = roots.countries;
    let mut acc = RowAccumulator::default();
    let mut source_proofs = SOURCE_PROOF_BUILD_SHA_870EBF | SOURCE_PROOF_COUNTRY_ROOTS_870EBF;
    let mut family_counts = Domain53FamilyCounts {
        country_roots: countries.len().min(u32::MAX as usize) as u32,
        ..Domain53FamilyCounts::default()
    };
    let mut staleness = Domain53DiscoveryStaleness::default();
    // W1 instrument (perf_rearch_20260706 T4): per-family wall timers. Each family
    // call site below is wrapped in Instant::now()/.elapsed(); the accumulator is
    // attached to the returned frame (success path only) and surfaced through the
    // collector's d53 DOMAIN_PERF diagnostic trailer (telemetry, never the folded
    // DATA snapshot). Observational: it changes no row, no wire byte, no status
    // flag, no error variant, and no control flow.
    let mut phase_times = Domain53PhaseTimes::default();
    // W1.1: attribute the pre-extract root-resolution wall (country-table
    // walk/validate) that the wrapper entry points ran before this call.
    phase_times.roots_us = roots_resolve_us;

    let wars_started = Instant::now();
    let (war_rows, war_total, war_stale_dropped) = match war_row_addrs {
        Some(addrs) => {
            let (rows, stale) =
                read_war_rows_from_validated_addrs(mem, module_base, &countries, addrs, &mut acc)?;
            (rows, addrs.len(), stale)
        }
        None => {
            let (rows, stale) =
                read_war_rows(mem, module_base, &countries, &discovery.war_hits, &mut acc)?;
            (rows, discovery.war_hits.iter().map(Vec::len).sum(), stale)
        }
    };
    phase_times.wars_us = duration_micros_u64_saturating(wars_started.elapsed());
    staleness.war_total = war_total;
    staleness.war_stale_dropped = war_stale_dropped;
    family_counts.war_scan_ranges = discovery.scan_range_count;

    // War-family staleness is decided HERE — right after the cheap war-address
    // re-read (dozens of addresses), BEFORE the expensive operative/operation
    // extraction below. On a stale-MAJORITY of the small validated war set the
    // war family is DOWNGRADED (not the whole frame): war rows are dropped and
    // `SOURCE_PROOF_WAR_ROOT_870EBF` is withheld so the payload builder sets
    // `STATUS_WAR_ROOT_UNVALIDATED`; the operative/operation families still
    // emit. Every SURVIVING war row here was already revalidated live
    // per-address (fail-closed row-by-row), so a MINORITY of churned wars keeps
    // a fully-validated war family. This defends the speed-5 case where a few
    // retired wars used to nuke the entire frame.
    //
    // IMPORTANT — no false war projection reaches the replay fold. The
    // per-country `at_war_count`/`casualties` are projected from the war rows
    // and live ON `CountryDiplomacyRow` alongside `faction_group`; the recorder
    // fold publishes all three unconditionally whenever a country row is present
    // (it does not consult `STATUS_WAR_ROOT_UNVALIDATED`). A downgraded frame
    // therefore must NOT emit `CountryDiplomacyRow` at all — a zeroed row would
    // be false "no wars" data, whereas an omitted row is honest "war state
    // unknown this frame". The country's faction-group signal is preserved
    // separately via the `country_faction_group` state event (which carries no
    // war fields), so only the war projection is withheld. See the lane README
    // war-root-unvalidated contract.
    // W1.1: time the war-degraded decision + war-projection fold — an untimed
    // span between the wars and faction_gate family timers. Observational only;
    // `war_degraded`/`war_projection` values and the `source_proofs |= ...` are
    // unchanged (the timer wraps them without altering any result).
    let war_projection_started = Instant::now();
    let war_degraded = staleness.war_degraded();
    let (war_rows, war_projection) = if war_degraded {
        (Vec::new(), BTreeMap::new())
    } else {
        source_proofs |= SOURCE_PROOF_WAR_ROOT_870EBF;
        let projection = war_projection_by_country(&war_rows);
        (war_rows, projection)
    };
    phase_times.war_projection_us =
        duration_micros_u64_saturating(war_projection_started.elapsed());
    let faction_gate_started = Instant::now();
    let faction_gate = read_faction_groups(mem, &countries)?;
    if faction_gate.validated {
        source_proofs |= SOURCE_PROOF_FACTION_GROUP_870EBF;
    }

    if faction_gate.validated {
        for country in &countries {
            let Some(&faction_group) = faction_gate.groups_by_country.get(&country.tag_db_id)
            else {
                continue;
            };
            // Country diplomacy row carries the war projection (at_war_count /
            // casualties). On a downgraded war family it is OMITTED entirely so
            // the fold never publishes a false zeroed projection; the
            // faction-group state event below still carries the valid
            // faction_group signal.
            if !war_degraded {
                let (at_war_count, casualties) = war_projection
                    .get(&country.tag_db_id)
                    .copied()
                    .unwrap_or((0, 0));
                acc.rows.countries.push(CountryDiplomacyRow {
                    tag_db_id: country.tag_db_id,
                    faction_group,
                    at_war_count,
                    casualties,
                });
            }
            acc.rows.state_events.push(state_event(
                country.tag_db_id,
                None,
                STATE_EVENT_SOURCE_COUNTRY_FACTION_GROUP,
                "country_faction_group",
                &format!("faction_group_{faction_group}"),
                i64::from(faction_group),
            ));
            acc.observe();
        }
    }
    phase_times.faction_gate_us = duration_micros_u64_saturating(faction_gate_started.elapsed());

    let relations_started = Instant::now();
    read_relation_rows(mem, module_base, current_game_hour, &countries, &mut acc)?;
    phase_times.relations_us = duration_micros_u64_saturating(relations_started.elapsed());
    let relation_rows =
        acc.rows.opinions.len() + acc.rows.relation_flags.len() + acc.rows.subjects.len();
    if relation_rows == 0 {
        return Err(ExtractError::InvalidRoot(
            "domain53_relations_empty_implausible",
        ));
    }
    family_counts.relation_rows = relation_rows.min(u32::MAX as usize) as u32;
    source_proofs |= SOURCE_PROOF_RELATION_ROOTS_870EBF;

    // CFactionSystem roster (id/type/name/members/leader). Non-fatal: a
    // structurally-invalid faction system leaves the roster empty (no proof),
    // never fails the whole frame.
    let faction_roster_started = Instant::now();
    let faction_rows = read_faction_rows(mem, module_base, game_state, &countries)?;
    phase_times.faction_roster_us =
        duration_micros_u64_saturating(faction_roster_started.elapsed());
    if !faction_rows.is_empty() {
        acc.rows.factions = faction_rows;
        source_proofs |= SOURCE_PROOF_FACTION_SYSTEM_870EBF;
    }

    let incoming_actions_started = Instant::now();
    read_incoming_action_queue_rows(mem, module_base, current_game_hour, &countries, &mut acc)?;
    phase_times.incoming_actions_us =
        duration_micros_u64_saturating(incoming_actions_started.elapsed());
    source_proofs |= SOURCE_PROOF_INCOMING_ACTION_QUEUE_870EBF;

    let agencies_started = Instant::now();
    let agency_roots = read_agency_rows(mem, module_base, &countries, &mut acc)?;
    phase_times.agencies_us = duration_micros_u64_saturating(agencies_started.elapsed());
    family_counts.agency_roots = agency_roots.len().min(u32::MAX as usize) as u32;
    source_proofs |= SOURCE_PROOF_AGENCY_ROOT_870EBF;

    let operatives_started = Instant::now();
    let operative_stale_dropped = read_operatives(
        mem,
        module_base,
        &countries,
        &discovery.operative_hits,
        &mut acc,
    )?;
    phase_times.operatives_us = duration_micros_u64_saturating(operatives_started.elapsed());
    staleness.operative_total = discovery.operative_hits.len();
    staleness.operative_stale_dropped = operative_stale_dropped;
    family_counts.operative_scan_ranges = discovery.scan_range_count;
    source_proofs |= SOURCE_PROOF_OPERATIVE_ROOT_870EBF;

    // Wave-5 running operations: consume the COperationInstance hits from the
    // same background discovery pass and re-validate each fresh. GENUINE liveness
    // failure (vtable gone / unreadable handle) is attributed to
    // operation_stale_dropped; candidate-rejection filtering (bad handle_type /
    // id, duplicate) is attributed SEPARATELY to operation_filtered. Only the
    // former is a staleness signal, and (A+C fix) it does NOT gate the frame —
    // operation rows are row-local fail-closed, so a high drop-rate just means
    // fewer operation rows, never the loss of the unrelated d53 families. The
    // family counts stay attributed separately from operatives so neither
    // family's churn masks the other.
    let operations_started = Instant::now();
    let (operation_instances, operation_drops) = read_operations(
        mem,
        module_base,
        &countries,
        &discovery.operation_instance_hits,
    )?;
    phase_times.operations_us = duration_micros_u64_saturating(operations_started.elapsed());
    acc.rows.operation_instances = operation_instances;
    staleness.operation_total = discovery.operation_instance_hits.len();
    staleness.operation_stale_dropped = operation_drops.stale_liveness;
    staleness.operation_filtered = operation_drops.filtered;
    family_counts.operation_candidates = discovery
        .operation_instance_hits
        .len()
        .min(u32::MAX as usize) as u32;
    family_counts.operation_emitted =
        acc.rows.operation_instances.len().min(u32::MAX as usize) as u32;
    family_counts.operation_stale_liveness =
        operation_drops.stale_liveness.min(u32::MAX as usize) as u32;
    family_counts.operation_filtered = operation_drops.filtered.min(u32::MAX as usize) as u32;

    let intel_networks_started = Instant::now();
    let intel_network_graphs = read_intel_network_graph_rows(
        mem,
        module_base,
        &countries,
        &discovery.strategic_operative_hits,
        &discovery.country_intel_network_hits,
    )?;
    phase_times.intel_networks_us =
        duration_micros_u64_saturating(intel_networks_started.elapsed());
    if let Some(intel_network_graphs) = intel_network_graphs {
        acc.rows.intel_network_graphs = intel_network_graphs;
        source_proofs |= SOURCE_PROOF_INTEL_NETWORK_GRAPH_870EBF;
    }

    // W1.1: the two per-agency aggregation loops (operative_count here,
    // cryptology_target_count below) are untimed O(agencies×operatives) /
    // O(agencies×cryptology) folds around the cryptology family timer. Attribute
    // both to `aggregation_us` (accumulated, since they straddle the cryptology
    // timer). Observational — the computed counts are unchanged.
    let aggregation_started = Instant::now();
    for agency in &mut acc.rows.agencies {
        agency.operative_count = acc
            .rows
            .operatives
            .iter()
            .filter(|row| row.owner_tag_db_id == agency.owner_tag_db_id)
            .count()
            .min(u32::MAX as usize) as u32;
    }
    phase_times.aggregation_us = duration_micros_u64_saturating(aggregation_started.elapsed());

    let cryptology_started = Instant::now();
    read_cryptology_rows(mem, module_base, &agency_roots, &mut acc)?;
    phase_times.cryptology_us = duration_micros_u64_saturating(cryptology_started.elapsed());
    if !agency_roots.is_empty() {
        source_proofs |= SOURCE_PROOF_CRYPTOLOGY_ROOT_870EBF;
    }
    let aggregation2_started = Instant::now();
    for agency in &mut acc.rows.agencies {
        agency.cryptology_target_count = acc
            .rows
            .cryptology
            .iter()
            .filter(|row| row.owner_tag_db_id == agency.owner_tag_db_id)
            .count()
            .min(u32::MAX as usize) as u32;
    }
    // W1.1: fold the second aggregation loop into the same segment bucket.
    phase_times.aggregation_us =
        phase_times
            .aggregation_us
            .saturating_add(duration_micros_u64_saturating(
                aggregation2_started.elapsed(),
            ));

    acc.rows.wars = war_rows;

    // Whole-frame fail-closed gate: ONLY the operative/operation families can
    // doom the frame (they have no per-family unvalidated wire status). The war
    // family was already handled above via `war_degraded()` — a stale war
    // majority downgrades just the war family (proof withheld ->
    // STATUS_WAR_ROOT_UNVALIDATED), it never reaches here. A dead/ancient sweep
    // makes operatives + operations stale too, so this still fails the whole
    // frame closed to SOURCE_UNAVAILABLE.
    if staleness.frame_degraded() {
        return Err(ExtractError::InvalidRoot(DOMAIN53_DISCOVERY_DEGRADED));
    }

    Ok(ExtractedDomain53Frame {
        rows: acc.rows,
        source_proofs,
        observed_rows: acc.observed_rows,
        skipped_rows: acc.skipped_rows,
        stale_dropped_total: staleness.stale_dropped_total(),
        family_counts,
        phase_times,
        // W3-a0 observational instrument: carry the relation-flag match-offset
        // histogram out to the collector's DOMAIN_PERF trailer. Diagnostic only.
        relflag_offsets: acc.relflag_offsets,
    })
}

fn verify_domain53_build_identity<M: Domain53Memory>(
    mem: &M,
    module_base: u64,
) -> Result<(), ExtractError> {
    verify_vtable_rtti_name(
        mem,
        module_base,
        profile::DOMAIN53_COUNTRY_VTABLE_RVA,
        RTTI_CCOUNTRY,
    )?;
    verify_vtable_rtti_name(
        mem,
        module_base,
        profile::DOMAIN53_AGENCY_VTABLE_RVA,
        RTTI_CINTELLIGENCE_AGENCY,
    )?;
    Ok(())
}

fn verify_vtable_rtti_name<M: Domain53Memory>(
    mem: &M,
    module_base: u64,
    vtable_rva: usize,
    expected_undecorated: &str,
) -> Result<(), ExtractError> {
    let vtable = add_addr(module_base, vtable_rva)?;
    let decorated = rtti_type_name_for_vtable(mem, module_base, vtable)?;
    let Some(undecorated) = undecorate_msvc_class_name(&decorated) else {
        return Err(ExtractError::Validation(format!(
            "domain53_rtti_name_not_class:{decorated}"
        )));
    };
    if undecorated != expected_undecorated {
        return Err(ExtractError::Validation(format!(
            "domain53_rtti_name_mismatch:{undecorated}!={expected_undecorated}"
        )));
    }
    Ok(())
}

fn rtti_type_name_for_vtable<M: Domain53Memory>(
    mem: &M,
    module_base: u64,
    vtable: u64,
) -> Result<String, ExtractError> {
    if !in_module(module_base, vtable) {
        return Err(ExtractError::InvalidRoot(
            "domain53_rtti_vtable_out_of_module",
        ));
    }
    let col_slot = vtable
        .checked_sub(8)
        .ok_or(ExtractError::InvalidRoot("domain53_rtti_vtable_underflow"))?;
    let col = mem.read_u64(col_slot)?;
    if !in_module(module_base, col) {
        return Err(ExtractError::InvalidRoot("domain53_rtti_col_out_of_module"));
    }
    let col_bytes = mem.read_bytes(col, 24)?;
    let signature = read_u32_from(&col_bytes, 0);
    let type_descriptor_rva = read_u32_from(&col_bytes, 0x0C);
    let class_descriptor_rva = read_u32_from(&col_bytes, 0x10);
    let self_rva = read_u32_from(&col_bytes, 0x14);
    if signature > 1 {
        return Err(ExtractError::InvalidRoot("domain53_rtti_col_signature"));
    }
    if signature == 1 && module_base.saturating_add(u64::from(self_rva)) != col {
        return Err(ExtractError::InvalidRoot("domain53_rtti_col_self_mismatch"));
    }
    if type_descriptor_rva == 0 || class_descriptor_rva == 0 {
        return Err(ExtractError::InvalidRoot("domain53_rtti_col_missing_refs"));
    }
    let type_descriptor = module_base
        .checked_add(u64::from(type_descriptor_rva))
        .ok_or(ExtractError::InvalidRoot(
            "domain53_rtti_type_descriptor_overflow",
        ))?;
    if !in_module(module_base, type_descriptor) {
        return Err(ExtractError::InvalidRoot(
            "domain53_rtti_type_descriptor_out_of_module",
        ));
    }
    read_cstr(
        mem,
        add_addr(type_descriptor, 0x10)?,
        DOMAIN53_RTTI_MAX_NAME_LEN,
    )
}

fn read_u32_from(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("u32 byte count"),
    )
}

fn read_cstr<M: Domain53Memory>(
    mem: &M,
    address: u64,
    max_len: usize,
) -> Result<String, ExtractError> {
    let bytes = mem.read_bytes(address, max_len)?;
    let Some(end) = bytes.iter().position(|byte| *byte == 0) else {
        return Err(ExtractError::InvalidRoot(
            "domain53_rtti_name_not_terminated",
        ));
    };
    let value = std::str::from_utf8(&bytes[..end])
        .map_err(|_| ExtractError::InvalidRoot("domain53_rtti_name_invalid_utf8"))?;
    if !value.starts_with(".?A") || value.chars().any(char::is_control) {
        return Err(ExtractError::InvalidRoot("domain53_rtti_name_invalid"));
    }
    Ok(value.to_string())
}

fn undecorate_msvc_class_name(name: &str) -> Option<&str> {
    name.strip_prefix(".?AV")?.strip_suffix("@@")
}

#[derive(Clone, Debug, Default)]
struct FactionGroupGate {
    validated: bool,
    groups_by_country: BTreeMap<u32, i32>,
}

fn read_faction_groups<M: Domain53Memory>(
    mem: &M,
    countries: &[CountryRoot],
) -> Result<FactionGroupGate, ExtractError> {
    let mut groups_by_country = BTreeMap::new();
    let mut group_members = BTreeMap::<i32, Vec<u32>>::new();
    let max_group = profile::DOMAIN53_MAX_COUNTRY_TABLE_SLOTS as i32;

    for country in countries {
        let value = match mem.read_i32(add_addr(
            country.country_ptr,
            profile::DOMAIN53_COUNTRY_FACTION_GROUP,
        )?) {
            Ok(value) if (0..=max_group).contains(&value) => value,
            _ => {
                return Ok(FactionGroupGate::default());
            }
        };
        groups_by_country.insert(country.tag_db_id, value);
        group_members
            .entry(value)
            .or_default()
            .push(country.tag_db_id);
    }

    let read_count = groups_by_country.len();
    let distinct_groups = group_members.len();
    let nonzero_groups = group_members.keys().filter(|group| **group > 0).count();
    let multi_member_nonzero_groups = group_members
        .iter()
        .filter(|(group, members)| **group > 0 && members.len() > 1)
        .count();

    let validated = read_count >= DOMAIN53_FACTION_PROBE_COUNTRY_FLOOR
        && read_count == countries.len()
        && distinct_groups > 0
        && nonzero_groups > 0
        && multi_member_nonzero_groups > 0;

    if validated {
        Ok(FactionGroupGate {
            validated: true,
            groups_by_country,
        })
    } else {
        Ok(FactionGroupGate::default())
    }
}

/// Read the CFactionSystem roster: `CGameState+0x3F8 -> CFactionSystem`, faction
/// pointer array at `+0x20`, live element count at `+0x28` (lo32=size,
/// hi32=capacity). Per faction: id/type at `+0x08` (id=HI32, type=LO32; real
/// factions carry type==88), name MSVC string at `+0x38`, members `CCountry*[]`
/// at `+0x58` with a u32 count at `+0x64`, leader = members[0]. Member country
/// pointers resolve to their tag_db_id (echo-checked against a validated
/// CCountry). Fail-closed: a torn/implausible header or member array yields no
/// factions (never a fabricated roster).
fn read_faction_rows<M: Domain53Memory>(
    mem: &M,
    module_base: u64,
    game_state: u64,
    countries: &[CountryRoot],
) -> Result<Vec<FactionRow>, ExtractError> {
    let valid_country_tags: BTreeSet<u32> =
        countries.iter().map(|country| country.tag_db_id).collect();

    let faction_system = match mem.read_u64(add_addr(
        game_state,
        profile::DOMAIN53_CGAMESTATE_FACTION_SYSTEM,
    )?) {
        Ok(value) if is_heap_ptr(value) => value,
        // No faction system resident — not an error; emit no roster.
        Ok(_) => return Ok(Vec::new()),
        Err(_) => return Ok(Vec::new()),
    };

    let array_base = match mem.read_u64(add_addr(faction_system, profile::DOMAIN53_FACTION_ARRAY)?)
    {
        Ok(value) if is_heap_ptr(value) => value,
        _ => return Ok(Vec::new()),
    };
    // +0x28 packs {size: u32, capacity: u32}.
    let packed_count =
        match mem.read_u64(add_addr(faction_system, profile::DOMAIN53_FACTION_COUNT)?) {
            Ok(value) => value,
            Err(_) => return Ok(Vec::new()),
        };
    let size = (packed_count & 0xFFFF_FFFF) as u32;
    let capacity = ((packed_count >> 32) & 0xFFFF_FFFF) as u32;
    if size == 0 || size > capacity || capacity > profile::DOMAIN53_FACTION_MAX_SLOTS {
        return Ok(Vec::new());
    }

    let expected_country_vtable = add_addr(module_base, profile::DOMAIN53_COUNTRY_VTABLE_RVA)?;
    let mut rows = Vec::new();
    let mut seen_ids = BTreeSet::new();
    // `size` is the EXACT live faction count (the array tail past it holds
    // garbage, but it is NOT inside `0..size`). Therefore every counted slot
    // MUST resolve to a well-formed faction; ANY anomaly is a torn/corrupt read,
    // so we FAIL CLOSED for the whole roster (return an empty Vec, no partial or
    // truncated roster is ever emitted). `read_faction_row` returns `None` on
    // such an anomaly.
    for slot in 0..size {
        let slot_addr = array_base
            .checked_add(u64::from(slot) * 8)
            .ok_or(ExtractError::InvalidRoot("domain53_faction_array_overflow"))?;
        match read_faction_row(
            mem,
            slot_addr,
            expected_country_vtable,
            &valid_country_tags,
            &mut seen_ids,
        )? {
            Some(row) => rows.push(row),
            // A single malformed faction in the counted range means the roster
            // read is torn — abandon the whole roster (fail closed).
            None => return Ok(Vec::new()),
        }
    }
    Ok(rows)
}

/// Read one CFaction from `slot_addr` (a `CFaction*` array slot). Returns
/// `Ok(None)` if the faction is malformed in any way (the caller treats that as
/// a torn roster and fails closed). Never fabricates.
fn read_faction_row<M: Domain53Memory>(
    mem: &M,
    slot_addr: u64,
    expected_country_vtable: u64,
    valid_country_tags: &BTreeSet<u32>,
    seen_ids: &mut BTreeSet<u32>,
) -> Result<Option<FactionRow>, ExtractError> {
    let faction_ptr = match mem.read_u64(slot_addr) {
        Ok(value) if is_heap_ptr(value) => value,
        _ => return Ok(None),
    };
    let raw_id = match mem.read_u64(add_addr(faction_ptr, profile::DOMAIN53_FACTION_ID)?) {
        Ok(value) => value,
        Err(_) => return Ok(None),
    };
    let faction_type = (raw_id & 0xFFFF_FFFF) as u32;
    let faction_id = ((raw_id >> 32) & 0xFFFF_FFFF) as u32;
    // Real factions carry type == 88 and a nonzero id. A different type inside
    // the counted range is a corrupt read.
    if faction_type != profile::DOMAIN53_FACTION_TYPE_EXPECTED || faction_id == 0 {
        return Ok(None);
    }
    if !seen_ids.insert(faction_id) {
        return Ok(None);
    }

    let member_count = match mem.read_u32(add_addr(
        faction_ptr,
        profile::DOMAIN53_FACTION_MEMBERS_SIZE,
    )?) {
        Ok(value) if value > 0 && value <= profile::DOMAIN53_FACTION_MAX_MEMBERS => value,
        _ => return Ok(None),
    };
    let members_base = match mem.read_u64(add_addr(faction_ptr, profile::DOMAIN53_FACTION_MEMBERS)?)
    {
        Ok(value) if is_heap_ptr(value) => value,
        _ => return Ok(None),
    };

    let mut leader_tag_db_id = 0u32;
    let mut member_tag_db_ids = Vec::with_capacity(member_count as usize);
    for member_slot in 0..member_count {
        let member_slot_addr = members_base.checked_add(u64::from(member_slot) * 8).ok_or(
            ExtractError::InvalidRoot("domain53_faction_member_overflow"),
        )?;
        let member_ptr = match mem.read_u64(member_slot_addr) {
            Ok(value) if is_heap_ptr(value) => value,
            _ => return Ok(None),
        };
        // Echo-check: the member must be a validated CCountry.
        match mem.read_u64(member_ptr) {
            Ok(vtable) if vtable == expected_country_vtable => {}
            _ => return Ok(None),
        }
        let member_tag = match mem.read_u32(add_addr(member_ptr, profile::DOMAIN53_OBJ_TAG_DB_ID)?)
        {
            Ok(value) if valid_tag_db_id(value) && valid_country_tags.contains(&value) => value,
            _ => return Ok(None),
        };
        // Leader is members[0] in engine order (captured BEFORE sorting the
        // emitted member list).
        if member_slot == 0 {
            leader_tag_db_id = member_tag;
        }
        member_tag_db_ids.push(member_tag);
    }
    if leader_tag_db_id == 0 {
        return Ok(None);
    }

    // Name is a display string; hash it (DIAQ/INGR string convention). A
    // missing/invalid name hashes a stable sentinel so downstream never sees a
    // fabricated value.
    let name_hash = match read_msvc_string(
        mem,
        add_addr(faction_ptr, profile::DOMAIN53_FACTION_NAME)?,
        128,
    ) {
        StringRead::Value(value) => crate::compact::stable_hash32(&value),
        StringRead::Missing => crate::compact::stable_hash32("faction_name_unknown"),
        StringRead::Invalid => crate::compact::stable_hash32("faction_name_invalid_utf8"),
    };

    // Emit the member list sorted+deduped (the FACS trailer decoder requires
    // strictly-increasing members); the leader is carried separately. A member
    // list that dedups shorter than the engine count means a duplicate member —
    // treat as corrupt.
    member_tag_db_ids.sort_unstable();
    member_tag_db_ids.dedup();
    if member_tag_db_ids.len() != member_count as usize
        || !member_tag_db_ids.contains(&leader_tag_db_id)
    {
        return Ok(None);
    }

    Ok(Some(FactionRow {
        faction_id,
        faction_type,
        name_hash,
        leader_tag_db_id,
        member_tag_db_ids,
    }))
}

fn resolve_country_roots<M: Domain53Memory>(
    mem: &M,
    module_base: u64,
    game_state: u64,
) -> Result<Vec<CountryRoot>, ExtractError> {
    let country_table = mem.read_u64(add_addr(
        game_state,
        profile::DOMAIN53_CGAMESTATE_COUNTRY_TABLE,
    )?)?;
    let country_count = mem.read_u32(add_addr(
        game_state,
        profile::DOMAIN53_CGAMESTATE_COUNTRY_COUNT,
    )?)?;
    if country_count == 0 || country_count > profile::DOMAIN53_MAX_COUNTRY_TABLE_SLOTS {
        return Err(ExtractError::InvalidCount {
            name: "domain53_country_count",
            value: country_count,
        });
    }
    if !is_heap_ptr(country_table) {
        return Err(ExtractError::InvalidRoot(
            "domain53_country_table_unavailable",
        ));
    }

    let expected_vtable = add_addr(module_base, profile::DOMAIN53_COUNTRY_VTABLE_RVA)?;
    let mut roots = Vec::new();
    let mut seen = BTreeSet::new();
    for slot in 0..country_count {
        let slot_addr = country_table
            .checked_add(u64::from(slot) * 8)
            .ok_or(ExtractError::InvalidRoot("domain53_country_table_overflow"))?;
        let Ok(country_ptr) = mem.read_u64(slot_addr) else {
            continue;
        };
        if country_ptr == 0 {
            continue;
        }
        if !is_heap_ptr(country_ptr) {
            continue;
        }
        let Ok(vtable) = mem.read_u64(country_ptr) else {
            continue;
        };
        if vtable != expected_vtable {
            continue;
        }
        let Ok(tag_db_id) = mem.read_u32(add_addr(country_ptr, profile::DOMAIN53_OBJ_TAG_DB_ID)?)
        else {
            continue;
        };
        if !valid_tag_db_id(tag_db_id) || !seen.insert(tag_db_id) {
            continue;
        }
        roots.push(CountryRoot {
            tag_db_id,
            country_ptr,
        });
    }
    Ok(roots)
}

/// Read one relation-table slot pointer (`u64[relation_table + tag_db_id*8]`),
/// byte-identically to the original per-slot `read_u64(entry_slot)` but sourced
/// from the once-per-src-country batched `table_bytes` slice when it is present.
///
/// Return shape mirrors the original inline code exactly:
///   * `Ok(Ok(ptr))`   -> the slot pointer value (heap/0/other handled by caller)
///   * `Ok(Err(_))`    -> a torn/failed slot read (caller does `acc.skip()`),
///                        identical to `read_u64` returning `Err`
///   * `Err(_)`         -> the `entry_slot` address overflow (propagated `?`),
///                        identical to the original `checked_add(...).ok_or(...)?`
///
/// Byte-identical argument: when `table_bytes` is `Some`, the whole array was
/// read in ONE all-or-nothing single-region `read_bytes`, so the 8 bytes at
/// `tag*8` are exactly the bytes `read_u64(relation_table + tag*8)` would have
/// copied (same address, same LE decode); and if the batch was `None`, we call
/// `mem.read_u64(entry_slot)` -- literally the original path.
/// T3 (perf_rearch_20260706): prefetch lookahead distance, in **iterations**,
/// for the d53 relation pair loop (~64k pairs/frame). While processing target
/// `i`, we prefetch the relation entry of target `i + PREFETCH_DIST_RELATION`.
/// The value is the DEFAULT: a live distance sweep {0, 8, 16, 32, 64} — measured
/// d53-alone and full-frame — is run from a SINGLE build via the runtime
/// override [`prefetch_dist_relation`] (env `BATTLE_PLAN_PREFETCH_DIST_RELATION`); this
/// constant is the fallback when the env var is absent/unparseable. Prefetch is
/// a hint only; this value affects timing, never rows/bytes/errors.
const PREFETCH_DIST_RELATION_DEFAULT: usize = 16;

/// T3 (perf_rearch_20260706): prefetch lookahead distance, in **iterations**,
/// for the flat address-slice candidate loops (`read_operatives`,
/// `read_operations`, `read_war_rows_from_validated_addrs`). While validating
/// `hits[i]`, we prefetch the candidate address `hits[i + dist]`. This is the
/// DEFAULT — the same single-build live {0, 8, 16, 32, 64} sweep applies via the
/// runtime override [`prefetch_dist_candidates`] (env
/// `BATTLE_PLAN_PREFETCH_DIST_CANDIDATES`); this constant is the fallback when the env
/// var is absent/unparseable. Hint only; no effect on rows/bytes/errors.
const PREFETCH_DIST_CANDIDATES_DEFAULT: usize = 32;

/// Environment variable overriding [`prefetch_dist_relation`] (the relation pair
/// loop lookahead). See [`parse_prefetch_dist`] for the parse contract.
const ENV_PREFETCH_DIST_RELATION: &str = "BATTLE_PLAN_PREFETCH_DIST_RELATION";

/// Environment variable overriding [`prefetch_dist_candidates`] (the flat
/// candidate loops' lookahead). See [`parse_prefetch_dist`] for the parse
/// contract.
const ENV_PREFETCH_DIST_CANDIDATES: &str = "BATTLE_PLAN_PREFETCH_DIST_CANDIDATES";

/// Pure parse for a runtime prefetch-distance override, factored out so the
/// contract is unit-testable without mutating the process environment (parallel
/// test env mutation is unsafe).
///
/// `raw` is the raw env-var value (`None` when unset). Returns `default` when
/// `raw` is `None`, empty, or does not parse as a `usize` (garbage). A parsed
/// value is returned as-is, **including `0`** — a distance of `0` is the
/// meaningful "prefetch OFF" arm of the sweep (the caller skips the lookahead
/// block entirely, issuing no `prefetch_read`). Perf-only: the result only
/// changes which non-faulting prefetch hints are issued, never rows/bytes/errors.
fn parse_prefetch_dist(raw: Option<&str>, default: usize) -> usize {
    match raw {
        Some(s) => s.trim().parse::<usize>().unwrap_or(default),
        None => default,
    }
}

/// Runtime lookahead distance for the d53 relation pair loop, read ONCE from
/// `BATTLE_PLAN_PREFETCH_DIST_RELATION` on first use and cached in a `OnceLock` (the env
/// is NOT read per-pair — that would perturb the measurement). Falls back to
/// [`PREFETCH_DIST_RELATION_DEFAULT`] (16) when the env var is absent/garbage. A
/// cached value of `0` DISABLES prefetch for the relation loop (the caller skips
/// the whole lookahead block, issuing no hint). Hint-only; default is identical
/// to the pre-override behavior.
fn prefetch_dist_relation() -> usize {
    static CACHED: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *CACHED.get_or_init(|| {
        parse_prefetch_dist(
            std::env::var(ENV_PREFETCH_DIST_RELATION).ok().as_deref(),
            PREFETCH_DIST_RELATION_DEFAULT,
        )
    })
}

/// Runtime lookahead distance for the flat candidate loops (`read_operatives`,
/// `read_operations`, `read_war_rows_from_validated_addrs`), read ONCE from
/// `BATTLE_PLAN_PREFETCH_DIST_CANDIDATES` on first use and cached in a `OnceLock` (never
/// per-candidate). Falls back to [`PREFETCH_DIST_CANDIDATES_DEFAULT`] (32) when
/// the env var is absent/garbage. A cached value of `0` DISABLES prefetch for
/// those loops (the caller skips the lookahead block). Hint-only; default is
/// identical to the pre-override behavior.
fn prefetch_dist_candidates() -> usize {
    static CACHED: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *CACHED.get_or_init(|| {
        parse_prefetch_dist(
            std::env::var(ENV_PREFETCH_DIST_CANDIDATES).ok().as_deref(),
            PREFETCH_DIST_CANDIDATES_DEFAULT,
        )
    })
}

fn relation_slot_ptr<M: Domain53Memory>(
    mem: &M,
    table_bytes: Option<&[u8]>,
    relation_table: u64,
    target_tag_db_id: u32,
) -> Result<Result<u64, ExtractError>, ExtractError> {
    let entry_slot = relation_table
        .checked_add(u64::from(target_tag_db_id) * 8)
        .ok_or(ExtractError::InvalidRoot(
            "domain53_relation_table_overflow",
        ))?;
    if let Some(bytes) = table_bytes {
        let lo = (target_tag_db_id as usize) * 8;
        // The batch length is `(max_target_tag+1)*8` and every target tag is
        // `<= max_target_tag`, so `lo + 8 <= bytes.len()`. Guard defensively and
        // fall back to a direct read on the (unreachable) short-slice case rather
        // than panic-indexing.
        if let Some(slot) = bytes.get(lo..lo + 8) {
            let value = u64::from_le_bytes(slot.try_into().expect("relation slot u64"));
            return Ok(Ok(value));
        }
    }
    Ok(mem.read_u64(entry_slot))
}

fn read_relation_rows<M: Domain53Memory>(
    mem: &M,
    module_base: u64,
    current_game_hour: u32,
    countries: &[CountryRoot],
    acc: &mut RowAccumulator,
) -> Result<(), ExtractError> {
    // The relation-flag vtables (13 kinds) used to classify each sub-object found in
    // the per-pair `pdx::vector<CRelation*>` container (see `read_relation_container`).
    let relation_vtables = relation_vtable_kinds(module_base)?;
    // T2 deliverable (c): `seen_opinions` REMOVED. `resolve_country_roots`
    // dedups countries by `tag_db_id` (its `seen.insert(tag_db_id)` gate, only
    // pushing a `CountryRoot` on first sight of a tag), so every element of
    // `countries` has a UNIQUE `tag_db_id` and `countries` is immutable through
    // this function. The `for src in countries { for target in countries }`
    // double loop therefore visits each ordered `(src.tag_db_id,
    // target.tag_db_id)` pair AT MOST ONCE — exactly one element carries each tag
    // — so `seen_opinions.insert((src.tag, target.tag))` ALWAYS returned `true`
    // and never deduplicated anything. Dropping it is behavior-identical. (`self`
    // pairs are skipped by the `src.tag == target.tag` guard, unchanged.)
    // T2 deliverable (d): precompute the closed opinion-string domain once per
    // call. `opinion` is validated into `-MAX_OPINION_ABS..=MAX_OPINION_ABS`
    // (±200 => 401 values) before use, so the `format!("opinion_{opinion}")`
    // string state_event argument comes from a tiny fixed set. Build all 401
    // strings once here and index them by `(opinion + MAX_OPINION_ABS) as usize`
    // in the pair loop instead of allocating a fresh `String` per pair. The
    // indexed string is byte-for-byte what `format!("opinion_{opinion}")`
    // produced (same signed decimal formatting), so `stable_hash32` of it —
    // the only thing `state_event` keeps — is identical.
    let opinion_labels: Vec<String> = (-MAX_OPINION_ABS..=MAX_OPINION_ABS)
        .map(|opinion| format!("opinion_{opinion}"))
        .collect();
    let mut seen_flags = BTreeSet::new();
    let mut seen_subjects = BTreeSet::new();

    // T2 deliverable (e): reserve the row vectors before the pair loop. Most
    // (src,target) pairs carry a non-default opinion, so the opinion / matching
    // state_event vectors grow to ~country_count^2 in the dense case; a modest
    // up-front reserve avoids repeated reallocation without over-committing.
    // Bounded so a large country table cannot request an absurd allocation.
    let pair_estimate = countries.len().saturating_mul(countries.len()).min(1 << 16);
    acc.rows.opinions.reserve(pair_estimate);
    // Each emitted opinion also pushes one state_event; flag state_events are
    // rare on top. Reserve the same modest headroom for the shared vector.
    acc.rows.state_events.reserve(pair_estimate);

    // T2 deliverable (e): hoist `max_target_tag` and the batched-table BYTE
    // LENGTH out of the per-src loop. `countries` is immutable across the loop,
    // so the highest target tag — and hence the `(max_tag+1)*8` span every
    // per-src batch read wants — is identical for every `src`. Only the actual
    // `read_bytes(relation_table, table_len)` stays inside the loop, because
    // `relation_table` is per-src. Computing `table_len` once is byte-neutral:
    // the same `Option<usize>` is fed to the same per-src read.
    let max_target_tag = countries
        .iter()
        .map(|country| country.tag_db_id)
        .max()
        .unwrap_or(0);
    let table_len: Option<usize> = u64::from(max_target_tag)
        .checked_add(1)
        .and_then(|slots| slots.checked_mul(8))
        .and_then(|len| usize::try_from(len).ok());

    // T3 sweep (perf_rearch_20260706): resolve the relation-loop prefetch
    // distance ONCE per call (cached OnceLock env read inside — NOT read
    // per-pair, which would perturb the measurement). `0` == prefetch OFF for
    // this loop (the per-target lookahead block below is skipped entirely, so no
    // `prefetch_read` is issued). Default (no env) == 16, identical to before.
    let prefetch_dist_relation = prefetch_dist_relation();

    for src in countries {
        let manager_addr = add_addr(src.country_ptr, profile::DOMAIN53_RELATION_MANAGER)?;
        let manager = match mem.read_u64(manager_addr) {
            Ok(value) if is_heap_ptr(value) => value,
            Ok(0) => continue,
            _ => {
                acc.skip();
                continue;
            }
        };
        let relation_table =
            match mem.read_u64(add_addr(manager, profile::DOMAIN53_RELATION_TABLE)?) {
                Ok(value) if is_heap_ptr(value) => value,
                _ => {
                    acc.skip();
                    continue;
                }
            };
        // P4c-2 read-count lever: the relation table is a contiguous
        // `tag_db_id -> entry_ptr` u64 array indexed by target.tag_db_id*8. The
        // old inner loop did one `read_u64(entry_slot)` PER (src,target) pair
        // (~470*470 = ~220k syscalls/frame). Read the WHOLE array ONCE per src
        // country (bounded by the max valid target tag) and index the pointers in
        // memory. The batch span [0, (max_tag+1)*8) is exactly bounded by the
        // highest slot any per-target `read_u64(relation_table + tag*8)` touches
        // (it also covers the self-slot and gaps the loop skips, but never extends
        // past that highest slot). `read_bytes` is all-or-nothing and single-
        // region: on success the whole array is inside ONE readable region, so
        // every USED slot subrange is readable and NO per-slot `read_u64` could
        // have failed -- the slice value at `tag*8` is byte-identical to
        // `read_u64(relation_table + tag*8)` (same address, same LE bytes). On ANY
        // short/failed batch read we fall
        // back to `relation_slot_ptr`, which reproduces the exact per-slot
        // `read_u64` (including its `acc.skip()` on a torn slot). A failed batch
        // read here counts NO skip on its own -- the per-slot fallback owns the
        // skip accounting, byte-identical to the original. (`max_target_tag` and
        // the `(max_tag+1)*8` byte length are hoisted above the src loop — see
        // T2 deliverable (e); only the per-src `read_bytes` stays here because
        // `relation_table` is per-src.)
        let table_bytes = table_len.and_then(|len| mem.read_bytes(relation_table, len).ok());
        for (target_index, target) in countries.iter().enumerate() {
            // T3 (perf_rearch_20260706): prefetch-ahead (MLP). While this
            // iteration walks the ~832-byte entry of `target`, software-prefetch
            // the entry of the country PREFETCH_DIST_RELATION positions ahead so
            // its DRAM latency overlaps the current work. This adds NO guarded
            // read: the future entry pointer is resolved ONLY by reading the
            // already-materialized immutable `table_bytes` slice at the future
            // country's `tag_db_id*8` slot — the same slice, same `tag*8..+8`
            // window and `u64::from_le_bytes` decode that `relation_slot_ptr`'s
            // slice path uses. When `table_bytes` is None (batch read failed) we
            // skip prefetching entirely — we never issue a new `read_u64` to look
            // ahead. `prefetch_read` re-checks `is_heap_ptr` on the exact address
            // it hands the intrinsic, so no unfiltered/non-canonical address can
            // reach `_mm_prefetch`. The lookahead is a pure hint: for most
            // upcoming targets it warms exactly the lines the later
            // `relation_slot_ptr`-resolved read will touch, but it does NOT
            // attempt to match that read 1:1 — e.g. when the future country is
            // `src` itself the loop will `continue` at the self-pair guard below
            // and never read that entry; prefetching it is simply a harmless warm
            // (it changes no read and no row). Removing this whole block leaves
            // rows/wire-bytes/errors byte-identical.
            // When the runtime distance is 0 (prefetch OFF arm of the sweep),
            // skip the whole lookahead block — no `prefetch_read` is issued and
            // no extra work is done (`future` is never resolved).
            if prefetch_dist_relation != 0 {
                if let Some(bytes) = table_bytes.as_deref() {
                    // `checked_add` so a pathological runtime distance (e.g.
                    // `BATTLE_PLAN_PREFETCH_DIST_RELATION=usize::MAX`) can never overflow
                    // the index — on overflow we simply skip the lookahead (None),
                    // exactly like an out-of-range `.get`. Keeps control flow
                    // identical to the const path for every real distance.
                    if let Some(future) = target_index
                        .checked_add(prefetch_dist_relation)
                        .and_then(|i| countries.get(i))
                    {
                        let lo = (future.tag_db_id as usize) * 8;
                        if let Some(slot) = bytes.get(lo..lo + 8) {
                            let future_entry =
                                u64::from_le_bytes(slot.try_into().expect("relation slot u64"));
                            // Two useful lines of the ~832 B (13-cache-line) entry:
                            //   +DOMAIN53_RELATION_OPINION (0x300, the FIRST field this
                            //   loop reads per pair — offset >= 64, so we prefetch
                            //   entry+0x300 rather than entry+0, per the T3 spec) and
                            //   +DOMAIN53_RELATION_SUBJECT_PTR (0x288). The hardware
                            // stream prefetcher follows the rest of the blob once the
                            // first touched line is resolved, so we deliberately do
                            // NOT prefetch all 13 lines. `checked_add` guards u64 wrap;
                            // `prefetch_read` applies the `is_heap_ptr` gate to the
                            // final (base+offset) address.
                            if let Some(opinion_line) =
                                future_entry.checked_add(profile::DOMAIN53_RELATION_OPINION as u64)
                            {
                                prefetch_read(opinion_line);
                            }
                            if let Some(subject_line) = future_entry
                                .checked_add(profile::DOMAIN53_RELATION_SUBJECT_PTR as u64)
                            {
                                prefetch_read(subject_line);
                            }
                        }
                    }
                }
            }
            if src.tag_db_id == target.tag_db_id {
                continue;
            }
            // Fetch the entry pointer from the batched slice when present (the
            // common case), else per-slot -- both yield the SAME `Result` shape so
            // the match arms below (heap => use, 0 => quiet continue, torn read =>
            // skip) are byte-identical to the original per-slot `read_u64`. The
            // `entry_slot` overflow guard is preserved inside `relation_slot_ptr`
            // (propagated as the same `ExtractError`).
            let relation_entry = match relation_slot_ptr(
                mem,
                table_bytes.as_deref(),
                relation_table,
                target.tag_db_id,
            )? {
                Ok(value) if is_heap_ptr(value) => value,
                Ok(0) => continue,
                _ => {
                    acc.skip();
                    continue;
                }
            };
            match mem.read_i32(add_addr(
                relation_entry,
                profile::DOMAIN53_RELATION_OPINION,
            )?) {
                Ok(opinion) if (-MAX_OPINION_ABS..=MAX_OPINION_ABS).contains(&opinion) => {
                    // T2 (c): the removed `seen_opinions` guard always fired
                    // exactly once per (src,target) pair (unique tags => unique
                    // pairs), so the push is now unconditional — identical rows.
                    acc.rows.opinions.push(OpinionRow {
                        src_tag_db_id: src.tag_db_id,
                        target_tag_db_id: target.tag_db_id,
                        opinion,
                    });
                    // T2 (d): index the precomputed label instead of
                    // `format!("opinion_{opinion}")`. `opinion` is in
                    // `-MAX_OPINION_ABS..=MAX_OPINION_ABS` (match guard), so
                    // `(opinion + MAX_OPINION_ABS)` is in `0..=2*MAX_OPINION_ABS`,
                    // a valid index into the 401-entry `opinion_labels`; the label
                    // is byte-for-byte the old `format!` output, so `stable_hash32`
                    // (the only value `state_event` retains) is identical.
                    let opinion_label = &opinion_labels[(opinion + MAX_OPINION_ABS) as usize];
                    acc.rows.state_events.push(state_event(
                        src.tag_db_id,
                        Some(target.tag_db_id),
                        STATE_EVENT_SOURCE_RELATION_OPINION,
                        "relation_opinion",
                        opinion_label,
                        i64::from(opinion),
                    ));
                    acc.observe();
                }
                Ok(_) | Err(_) => {
                    acc.skip();
                    continue;
                }
            }

            // Subject (puppet) relations are NOT embedded inline in the relation
            // object on this build: `CRelationStatus+0x288` is a POINTER to a
            // heap `CSubjectRelation`. The committed inline vtable scan therefore
            // never matched (0 subjects). Read the pointer hop directly, gated on
            // the CSubjectRelation vtable + a valid (overlord,subject) pair that
            // agrees with the current relation-table direction; emit exactly once
            // from the overlord's own table entry (the subject's mirrored entry
            // shares the same object and is skipped).
            read_subject_relation(
                mem,
                module_base,
                relation_entry,
                src.tag_db_id,
                target.tag_db_id,
                current_game_hour,
                acc,
                &mut seen_subjects,
                &mut seen_flags,
            );

            // Relation-flag sub-objects (non_aggression_pact, military_access,
            // docking_rights, guarantee, embargo, ...) are NOT inline in the `rel`
            // object: each is a heap `CRelation` object held in a `pdx::vector<CRelation*>`
            // whose header lives on `rel` (begin +0xD0, count +0xDC). The former inline
            // `[rel, rel+0x340)` vtable scan therefore matched ZERO of them (proven live:
            // relflag_total_matches=0 in both peace and war), silently dropping every
            // NAP/mil-access/docking/guarantee/embargo. Walk the container instead — this
            // CLOSES that data gap AND retires the ~53 MB/frame + ~6.7M-qword/frame dead
            // scan. See runs/perf_rearch_20260706/RELATION_CONTAINER_RE.md (RE, Codex-
            // endorsed) + RELATION_FLAG_GAP_LEDGER.md (no-duplication ledger).
            read_relation_container(
                mem,
                relation_entry,
                &relation_vtables,
                src.tag_db_id,
                target.tag_db_id,
                current_game_hour,
                acc,
                &mut seen_flags,
            );
        }
    }
    Ok(())
}

/// Read a subject (puppet) relation through the `CRelationStatus+0x288` pointer
/// hop and emit a subject row + its puppet relation flag. Called once per
/// relation-table entry `(src, target)`. The `CSubjectRelation` object is shared
/// between the overlord's and the subject's mirrored table entries, so we emit
/// ONLY from the overlord's own entry (`src == overlord && target == subject`);
/// the mirror direction is silently ignored. Fail-closed: absent pointer →
/// nothing; torn/short read or an object whose fields disagree with the
/// relation-table direction → skip (never fabricate).
#[allow(clippy::too_many_arguments)]
fn read_subject_relation<M: Domain53Memory>(
    mem: &M,
    module_base: u64,
    relation_entry: u64,
    src_tag_db_id: u32,
    target_tag_db_id: u32,
    current_game_hour: u32,
    acc: &mut RowAccumulator,
    seen_subjects: &mut BTreeSet<(u32, u32)>,
    seen_flags: &mut BTreeSet<(u32, u32, u16)>,
) {
    // The subject pointer at +0x288 is absent for the vast majority of relation
    // entries (only actual overlord/subject pairs carry one). Resolve the field
    // address fail-closed (an overflow here would be a corrupt relation_entry).
    let subject_ptr_addr = match add_addr(relation_entry, profile::DOMAIN53_RELATION_SUBJECT_PTR) {
        Ok(value) => value,
        Err(_) => {
            acc.skip();
            return;
        }
    };
    let subject_ptr = match mem.read_u64(subject_ptr_addr) {
        Ok(value) => value,
        // A torn/short read of the pointer field is a real failure — count it so
        // the fail-closed majority gate can react (do NOT silently omit).
        Err(_) => {
            acc.skip();
            return;
        }
    };
    // A null / non-heap pointer is the normal non-puppet case — return quietly
    // WITHOUT a skip so every non-puppet relation does not perturb the gate.
    if !is_heap_ptr(subject_ptr) {
        return;
    }
    // Primary type guard: the pointed object must be a CSubjectRelation.
    let expected_vtable = match add_addr(module_base, profile::DOMAIN53_REL_SUBJECT_VTABLE_RVA) {
        Ok(value) => value,
        Err(_) => return,
    };
    match mem.read_u64(subject_ptr) {
        Ok(vtable) if vtable == expected_vtable => {}
        // A readable but wrong vtable is a TYPE REJECTION, not a torn read: this
        // +0x288 slot simply is not a CSubjectRelation for this relation kind.
        // Return quietly (do NOT count a skip) — same as an inline flag-scan
        // vtable miss. Counting it would fail-close the frame on benign
        // non-subject data.
        Ok(_) => return,
        // An UNreadable vtable at a heap pointer we already dereferenced is a
        // torn read → count a skip so the fail-closed gate can react.
        Err(_) => {
            acc.skip();
            return;
        }
    }

    let read = (|| -> Result<SubjectRow, ExtractError> {
        let overlord = mem.read_u32(add_addr(subject_ptr, profile::DOMAIN53_SUBJECT_OVERLORD)?)?;
        let subject = mem.read_u32(add_addr(subject_ptr, profile::DOMAIN53_SUBJECT_SUBJECT)?)?;
        if !valid_tag_db_id(overlord) || !valid_tag_db_id(subject) || overlord == subject {
            return Err(ExtractError::InvalidRoot("domain53_subject_tag_invalid"));
        }
        // Direction consistency. The SAME CSubjectRelation object is reachable
        // from BOTH the overlord's table entry (src==overlord && target==subject)
        // and the subject's mirrored entry (src==subject && target==overlord).
        //   * overlord entry  -> emit (below).
        //   * mirror entry     -> benign expected duplicate: skip WITHOUT counting
        //     a skipped row (distinct sentinel, matched separately).
        //   * anything else    -> the +0x288 pointer of THIS relation entry points
        //     at a CSubjectRelation for an UNRELATED pair: that is a real
        //     inconsistency, so fall through to a counted skip.
        if src_tag_db_id == subject && target_tag_db_id == overlord {
            return Err(ExtractError::InvalidRoot("domain53_subject_direction_skip"));
        }
        if src_tag_db_id != overlord || target_tag_db_id != subject {
            return Err(ExtractError::InvalidRoot(
                "domain53_subject_direction_mismatch",
            ));
        }
        let state_db = mem.read_u64(add_addr(subject_ptr, profile::DOMAIN53_SUBJECT_STATE_DB)?)?;
        let state_hash = if is_heap_ptr(state_db) {
            match read_msvc_string(
                mem,
                add_addr(state_db, profile::DOMAIN53_SUBJECT_STATE_NAME)?,
                96,
            ) {
                StringRead::Value(value) => crate::compact::stable_hash32(&value),
                StringRead::Invalid => {
                    acc.skip();
                    crate::compact::stable_hash32("autonomy_state_invalid_utf8")
                }
                StringRead::Missing => crate::compact::stable_hash32("autonomy_state_unknown"),
            }
        } else {
            crate::compact::stable_hash32("autonomy_state_unknown")
        };
        // ratio (+0x60) has no melt oracle — carried UNVALIDATED, range-gated.
        let ratio = mem.read_i32(add_addr(subject_ptr, profile::DOMAIN53_SUBJECT_RATIO)?)?;
        if !(-MAX_FIXED_RAW..=MAX_FIXED_RAW).contains(&ratio) {
            return Err(ExtractError::InvalidRoot("domain53_subject_ratio_invalid"));
        }
        // start_date (+0x20) i32 engine-hour, RAW. A subject cannot start in the
        // future; gate strictly positive and no later than the current game hour
        // (with a small margin for live-read drift during capture).
        let start_date_raw =
            mem.read_i32(add_addr(subject_ptr, profile::DOMAIN53_SUBJECT_START_DATE)?)?;
        let clock_ceiling = i64::from(current_game_hour) + 24;
        if start_date_raw <= 0 || i64::from(start_date_raw) > clock_ceiling {
            return Err(ExtractError::InvalidRoot(
                "domain53_subject_start_date_invalid",
            ));
        }
        Ok(SubjectRow {
            overlord_tag_db_id: overlord,
            subject_tag_db_id: subject,
            autonomy_state_hash: state_hash,
            autonomy_ratio_raw: ratio,
            start_date_raw,
        })
    })();

    match read {
        Ok(row) => {
            if seen_subjects.insert((row.overlord_tag_db_id, row.subject_tag_db_id)) {
                // Synthesize the puppet relation flag (directed overlord->subject)
                // — the inline vtable scan never sees it, so it would otherwise be
                // silently dropped.
                let flag_key = (
                    row.overlord_tag_db_id,
                    row.subject_tag_db_id,
                    REL_KIND_PUPPET,
                );
                if seen_flags.insert(flag_key) {
                    acc.rows.relation_flags.push(RelationFlagRow {
                        src_tag_db_id: row.overlord_tag_db_id,
                        target_tag_db_id: row.subject_tag_db_id,
                        flag_kind: REL_KIND_PUPPET,
                        directed: true,
                    });
                    acc.rows.state_events.push(state_event(
                        row.overlord_tag_db_id,
                        Some(row.subject_tag_db_id),
                        STATE_EVENT_SOURCE_RELATION_FLAG,
                        &format!("relation_flag_kind_{REL_KIND_PUPPET}"),
                        "directed_true",
                        i64::from(REL_KIND_PUPPET),
                    ));
                    acc.observe();
                }
                acc.rows.state_events.push(StateEventRow {
                    owner_tag_db_id: row.overlord_tag_db_id,
                    target_tag_db_id: Some(row.subject_tag_db_id),
                    event_kind: STATE_EVENT_KIND_OBSERVED,
                    source_kind: STATE_EVENT_SOURCE_SUBJECT_AUTONOMY,
                    object_hash: row.autonomy_state_hash,
                    value_hash: crate::compact::stable_hash32(&format!(
                        "autonomy_ratio_{}",
                        row.autonomy_ratio_raw
                    )),
                    value_raw: i64::from(row.autonomy_ratio_raw),
                });
                acc.rows.subjects.push(row);
                acc.observe();
            }
        }
        // A direction skip is the expected mirror-entry case, not an error;
        // do not count it as a skipped row (that would trip the fail-closed
        // majority gate). Only true read/validation failures count as skips.
        Err(ExtractError::InvalidRoot("domain53_subject_direction_skip")) => {}
        Err(_) => acc.skip(),
    }
}

/// d53 owns these relation-flag kinds (the previously-dropped gap). EXCLUDED because
/// captured elsewhere: `REL_KIND_PUPPET` (d53, via the `+0x288` subject pointer hop),
/// `REL_KIND_MARKET_ACCESS_RIGHTS` (domain 43), `REL_KIND_AIR_BASE_ACCESS` (domain 30).
/// See runs/perf_rearch_20260706/RELATION_FLAG_GAP_LEDGER.md.
fn relation_kind_emitted_by_d53(kind: u16) -> bool {
    !matches!(
        kind,
        REL_KIND_PUPPET | REL_KIND_MARKET_ACCESS_RIGHTS | REL_KIND_AIR_BASE_ACCESS
    )
}

/// Walk the per-pair relation sub-object container — a `pdx::vector<CRelation*>` whose
/// header lives on the `rel` object (`begin +0xD0`, `cap +0xD8`, `count +0xDC`) — and emit
/// a [`RelationFlagRow`] plus start/end `state_events` for each active d53-owned
/// relation-flag kind (non_aggression_pact, military_access, docking_rights, naval_blockade,
/// equipment_purchase_contract_relation, guarantee, improve_relations, attache,
/// licence_production, embargo). Kinds owned by other domains (puppet/+0x288, market/d43,
/// air-base/d30) are COUNTED in the diagnostic but NOT emitted here.
///
/// Fail-closed like [`read_subject_relation`]: a torn read of a header field or of a
/// matched sub-object's fields counts a skip (so the majority gate can react); an
/// empty container (`count == 0`), a null element, or a non-relation (unknown-vtable)
/// sibling returns quietly WITHOUT a skip — the ~64k pairs with no sub-relations must not
/// perturb the gate. Never fabricates: a sub-object must carry a known relation vtable, a
/// valid tag pair, AND that pair must be exactly THIS bucket's `(src, target)` (in either
/// order) — a container entry for another pair is a real inconsistency and is skipped.
///
/// The pointer array is read as ONE bounded span, so a torn array costs a single structural
/// skip (not one per element). Dedup is by the sub-object's OWN `(first, second, kind)` via
/// the frame-global `seen_flags`: the same relation object is pushed into BOTH direction
/// buckets (`rel(A->B)` and `rel(B->A)`), so keying on its stored `first`/`second` emits it
/// exactly once regardless of which bucket the walk reaches first. Dedup is applied LAST so
/// a torn/invalid entry never poisons the key for a valid mirror.
/// See runs/perf_rearch_20260706/RELATION_CONTAINER_RE.md.
#[allow(clippy::too_many_arguments)]
fn read_relation_container<M: Domain53Memory>(
    mem: &M,
    relation_entry: u64,
    relation_vtables: &BTreeMap<u64, u16>,
    src_tag_db_id: u32,
    target_tag_db_id: u32,
    current_game_hour: u32,
    acc: &mut RowAccumulator,
    seen_flags: &mut BTreeSet<(u32, u32, u16)>,
) {
    // Vector header {begin @+0xD0, cap @+0xD8, count @+0xDC}. Read COUNT first so the
    // empty vs corrupt distinction is exact. A torn read of a header field on this heap
    // object (whose opinion @+0x300 already read cleanly) is a real failure -> skip.
    let count = match add_addr(relation_entry, profile::DOMAIN53_RELATION_SUBREL_COUNT)
        .and_then(|addr| mem.read_i32(addr))
    {
        Ok(value) => value,
        Err(_) => {
            acc.skip();
            return;
        }
    };
    // Empty vector: the COMMON case for the vast majority of pairs. Quiet, NO skip.
    if count == 0 {
        return;
    }
    // A negative or absurd count is a garbage/torn header, not an empty vector -> skip.
    // (`count == 0` already returned above, so valid live counts are `1..=MAX`.)
    if !(1..=profile::DOMAIN53_RELATION_SUBREL_MAX_COUNT).contains(&count) {
        acc.skip();
        return;
    }
    // With `count > 0`, `begin` MUST point at a heap array. A null / non-heap begin here
    // is an inconsistent (torn) header -> skip (NOT the quiet empty case).
    let begin = match add_addr(relation_entry, profile::DOMAIN53_RELATION_SUBREL_BEGIN)
        .and_then(|addr| mem.read_u64(addr))
    {
        Ok(value) => value,
        Err(_) => {
            acc.skip();
            return;
        }
    };
    if !is_heap_ptr(begin) {
        acc.skip();
        return;
    }
    // Capacity sanity: a live pdx::vector has `count <= cap` and a bounded cap.
    let cap = match add_addr(relation_entry, profile::DOMAIN53_RELATION_SUBREL_CAP)
        .and_then(|addr| mem.read_i32(addr))
    {
        Ok(value) => value,
        Err(_) => {
            acc.skip();
            return;
        }
    };
    if cap < count || cap > profile::DOMAIN53_RELATION_SUBREL_MAX_COUNT {
        acc.skip();
        return;
    }
    // Read the whole `CRelation*` array in ONE bounded span. A torn/short array then costs
    // exactly one structural skip instead of up to `count` per-slot skips (a skip flood
    // from a single bad header could otherwise trip the majority gate = false frame loss).
    // `is_heap_ptr(begin)` bounds `begin < 0x7FF0_0000_0000` and `array_len <= 2048`, so
    // `begin + array_len` cannot overflow; a short/unreadable span fails the read.
    let array_len = (count as usize) * 8;
    let array = match mem.read_bytes(begin, array_len) {
        Ok(bytes) => bytes,
        Err(_) => {
            acc.skip();
            return;
        }
    };
    for chunk in array.chunks_exact(8) {
        let sub = u64::from_le_bytes(chunk.try_into().expect("8-byte pointer slot"));
        // A null / non-heap element is benign padding; skip quietly.
        if !is_heap_ptr(sub) {
            continue;
        }
        // Identify the kind by the vtable at `sub+0`. An unknown vtable is a
        // heterogeneous-container sibling (CWarRelation / CLendLeaseRelation / ...) or a
        // torn object -> skip quietly (not an error, not a d53 kind).
        let vptr = match mem.read_u64(sub) {
            Ok(value) => value,
            Err(_) => {
                acc.skip();
                continue;
            }
        };
        let kind = match relation_vtables.get(&vptr).copied() {
            Some(value) => value,
            None => continue,
        };
        // Diagnostic: count EVERY known-kind container entry (incl. the non-d53 kinds) so
        // the live per-kind totals cross-check against the melt oracle.
        acc.relflag_offsets.record_kind_count(kind);
        // Skip kinds captured elsewhere (puppet via +0x288, market -> d43, air-base -> d30).
        if !relation_kind_emitted_by_d53(kind) {
            continue;
        }
        let first = match add_addr(sub, profile::DOMAIN53_REL_BASE_FIRST)
            .and_then(|addr| mem.read_u32(addr))
        {
            Ok(value) => value,
            Err(_) => {
                acc.skip();
                continue;
            }
        };
        let second = match add_addr(sub, profile::DOMAIN53_REL_BASE_SECOND)
            .and_then(|addr| mem.read_u32(addr))
        {
            Ok(value) => value,
            Err(_) => {
                acc.skip();
                continue;
            }
        };
        // Matched vtable but garbage tags = a real inconsistency (never fabricate).
        if !valid_tag_db_id(first) || !valid_tag_db_id(second) || first == second {
            acc.skip();
            continue;
        }
        // Bucket-pair consistency: this container belongs to the (src, target) pair, so a
        // sub-object's stored `(first, second)` MUST be that pair (in either order). An
        // entry for a different pair means a torn `begin` reached another pair's object (or
        // a genuine engine inconsistency) — skip, never emit a fabricated wrong-pair row.
        let matches_bucket = (first == src_tag_db_id && second == target_tag_db_id)
            || (first == target_tag_db_id && second == src_tag_db_id);
        if !matches_bucket {
            acc.skip();
            continue;
        }
        // start_date (engine-hour, RAW). `Init` always writes a real start; the base-ctor
        // sentinel here means uninitialized/torn. Gate strictly positive and no later than
        // the current game hour (small live-read-drift margin), like the subject reader.
        let start_raw = match add_addr(sub, profile::DOMAIN53_REL_BASE_START_DATE)
            .and_then(|addr| mem.read_i32(addr))
        {
            Ok(value) => value,
            Err(_) => {
                acc.skip();
                continue;
            }
        };
        let clock_ceiling = i64::from(current_game_hour) + 24;
        if start_raw == profile::DOMAIN53_REL_DATE_SENTINEL
            || start_raw <= 0
            || i64::from(start_raw) > clock_ceiling
        {
            acc.skip();
            continue;
        }
        // end_date: the sentinel (or any non-positive/torn value) means "no end_date"
        // (None) — the relation's existence + start are still valid, so we keep the row but
        // emit no end event rather than fabricate a garbage end. A real positive end may be
        // in the FUTURE (the relation's expiry), so it is NOT clock-ceiling gated.
        let end_raw = match add_addr(sub, profile::DOMAIN53_REL_BASE_END_DATE)
            .and_then(|addr| mem.read_i32(addr))
        {
            Ok(value) if value != profile::DOMAIN53_REL_DATE_SENTINEL && value > 0 => Some(value),
            Ok(_) => None,
            Err(_) => {
                acc.skip();
                continue;
            }
        };
        // Dedup LAST (after all reads/validation).
        if !seen_flags.insert((first, second, kind)) {
            continue;
        }
        acc.rows.relation_flags.push(RelationFlagRow {
            src_tag_db_id: first,
            target_tag_db_id: second,
            flag_kind: kind,
            directed: true,
        });
        acc.rows.state_events.push(state_event(
            first,
            Some(second),
            STATE_EVENT_SOURCE_RELATION_FLAG,
            &format!("relation_flag_kind_{kind}"),
            "directed_true",
            i64::from(kind),
        ));
        acc.rows.state_events.push(state_event(
            first,
            Some(second),
            STATE_EVENT_SOURCE_RELATION_FLAG,
            &format!("relation_flag_start_{kind}"),
            "start_date_raw",
            i64::from(start_raw),
        ));
        if let Some(end) = end_raw {
            acc.rows.state_events.push(state_event(
                first,
                Some(second),
                STATE_EVENT_SOURCE_RELATION_FLAG,
                &format!("relation_flag_end_{kind}"),
                "end_date_raw",
                i64::from(end),
            ));
        }
        acc.observe();
    }
}

fn read_incoming_action_queue_rows<M: Domain53Memory>(
    mem: &M,
    module_base: u64,
    current_game_hour: u32,
    countries: &[CountryRoot],
    acc: &mut RowAccumulator,
) -> Result<(), ExtractError> {
    let action_vtable = add_addr(module_base, profile::DOMAIN53_INCOMING_ACTION_VTABLE_RVA)?;
    let valid_tags = countries
        .iter()
        .map(|country| country.tag_db_id)
        .collect::<BTreeSet<_>>();
    let mut seen_actions = BTreeSet::new();

    for recipient in countries {
        let queue = match mem.read_u64(add_addr(
            recipient.country_ptr,
            profile::DOMAIN53_INCOMING_ACTION_QUEUE,
        )?) {
            Ok(value) if is_heap_ptr(value) => value,
            Ok(_) => continue,
            Err(_) => {
                acc.skip();
                continue;
            }
        };
        let packed = match mem.read_u64(add_addr(
            queue,
            profile::DOMAIN53_INCOMING_ACTION_QUEUE_PACKED_SIZE_CAP,
        )?) {
            Ok(value) => value,
            Err(_) => {
                acc.skip();
                continue;
            }
        };
        let size = (packed & 0xFFFF_FFFF) as u32;
        let cap = (packed >> 32) as u32;
        if size > cap || cap > profile::DOMAIN53_MAX_INCOMING_ACTION_ROWS {
            acc.skip();
            continue;
        }
        if size == 0 {
            continue;
        }
        let owner_guard = match mem.read_u32(add_addr(
            queue,
            profile::DOMAIN53_INCOMING_ACTION_QUEUE_OWNER_GUARD,
        )?) {
            Ok(value) => value,
            Err(_) => {
                acc.skip();
                continue;
            }
        };
        if owner_guard != recipient.tag_db_id {
            acc.skip();
            continue;
        }
        let data = match mem.read_u64(add_addr(
            queue,
            profile::DOMAIN53_INCOMING_ACTION_QUEUE_DATA,
        )?) {
            Ok(value) if is_heap_ptr(value) => value,
            _ => {
                acc.skip();
                continue;
            }
        };
        for index in 0..size {
            let slot = data
                .checked_add(u64::from(index) * 8)
                .ok_or(ExtractError::InvalidRoot(
                    "domain53_incoming_action_queue_overflow",
                ))?;
            match read_incoming_action_row(
                mem,
                slot,
                recipient.tag_db_id,
                action_vtable,
                current_game_hour,
                &valid_tags,
            ) {
                Ok(row) if seen_actions.insert(row.action_id) => {
                    acc.rows.incoming_actions.push(row);
                    acc.observe();
                }
                Ok(_) => {
                    acc.skip();
                    continue;
                }
                Err(_) => {
                    acc.skip();
                    continue;
                }
            }
        }
    }
    acc.rows
        .incoming_actions
        .sort_by_key(|row| (row.recipient_tag_db_id, row.action_id));
    Ok(())
}

fn read_incoming_action_row<M: Domain53Memory>(
    mem: &M,
    slot: u64,
    expected_recipient: u32,
    action_vtable: u64,
    current_game_hour: u32,
    valid_tags: &BTreeSet<u32>,
) -> Result<IncomingActionQueueRow, ExtractError> {
    let action = mem.read_u64(slot)?;
    if !is_heap_ptr(action) || mem.read_u64(action)? != action_vtable {
        return Err(ExtractError::InvalidRoot(
            "domain53_incoming_action_vtable_invalid",
        ));
    }
    let handle = mem.read_u64(add_addr(action, profile::DOMAIN53_INCOMING_ACTION_HANDLE)?)?;
    let handle_type = (handle & 0xFFFF_FFFF) as u32;
    if handle_type != profile::DOMAIN53_INCOMING_ACTION_HANDLE_TYPE {
        return Err(ExtractError::InvalidRoot(
            "domain53_incoming_action_handle_type_invalid",
        ));
    }
    let action_id = (handle >> 32) as u32;
    if action_id == 0 || action_id > MAX_OPERATION_ID {
        return Err(ExtractError::InvalidRoot(
            "domain53_incoming_action_id_invalid",
        ));
    }
    let body = mem.read_u64(add_addr(action, profile::DOMAIN53_INCOMING_ACTION_BODY)?)?;
    if !is_heap_ptr(body) {
        return Err(ExtractError::InvalidRoot(
            "domain53_incoming_action_body_invalid",
        ));
    }
    let action_type = mem.read_i32(add_addr(body, profile::DOMAIN53_INCOMING_ACTION_BODY_TYPE)?)?;
    let actor = mem.read_u32(add_addr(body, profile::DOMAIN53_INCOMING_ACTION_ACTOR)?)?;
    let original_actor = mem.read_u32(add_addr(
        body,
        profile::DOMAIN53_INCOMING_ACTION_ORIGINAL_ACTOR,
    )?)?;
    let recipient = mem.read_u32(add_addr(body, profile::DOMAIN53_INCOMING_ACTION_RECIPIENT)?)?;
    let original_recipient = mem.read_u32(add_addr(
        body,
        profile::DOMAIN53_INCOMING_ACTION_ORIGINAL_RECIPIENT,
    )?)?;
    let date_hours = mem.read_u32(add_addr(body, profile::DOMAIN53_INCOMING_ACTION_DATE)?)?;
    let last_command_date_hours = mem.read_u32(add_addr(
        body,
        profile::DOMAIN53_INCOMING_ACTION_LAST_COMMAND_DATE,
    )?)?;
    if actor != original_actor
        || recipient != original_recipient
        || recipient != expected_recipient
        || !valid_tag_db_id(actor)
        || !valid_tag_db_id(recipient)
        || !valid_tags.contains(&actor)
        || !valid_tags.contains(&recipient)
    {
        return Err(ExtractError::InvalidRoot(
            "domain53_incoming_action_tag_guard_invalid",
        ));
    }
    if date_hours > current_game_hour
        || last_command_date_hours > current_game_hour
        || date_hours > MAX_ACTION_DATE_HOURS
        || last_command_date_hours > MAX_ACTION_DATE_HOURS
    {
        return Err(ExtractError::InvalidRoot(
            "domain53_incoming_action_date_invalid",
        ));
    }
    Ok(IncomingActionQueueRow {
        recipient_tag_db_id: recipient,
        actor_tag_db_id: actor,
        action_id,
        action_type,
        date_hours,
        last_command_date_hours,
    })
}

fn read_agency_rows<M: Domain53Memory>(
    mem: &M,
    _module_base: u64,
    countries: &[CountryRoot],
    acc: &mut RowAccumulator,
) -> Result<Vec<AgencyRootRead>, ExtractError> {
    let expected_vtable = add_addr(mem.module_base(), profile::DOMAIN53_AGENCY_VTABLE_RVA)?;
    let mut agency_roots = Vec::new();
    for country in countries {
        let agency_ptr = match mem.read_u64(add_addr(
            country.country_ptr,
            profile::DOMAIN53_COUNTRY_AGENCY_PTR,
        )?) {
            Ok(0) => continue,
            Ok(value) if is_heap_ptr(value) => value,
            Ok(_) | Err(_) => {
                acc.skip();
                continue;
            }
        };
        let Ok(vtable) = mem.read_u64(agency_ptr) else {
            acc.skip();
            continue;
        };
        if vtable != expected_vtable {
            acc.skip();
            continue;
        }
        let defense_raw =
            match mem.read_i32(add_addr(agency_ptr, profile::DOMAIN53_AGENCY_DEFENSE)?) {
                Ok(value) if (-MAX_FIXED_RAW..=MAX_FIXED_RAW).contains(&value) => value,
                _ => {
                    acc.skip();
                    continue;
                }
            };
        let upgrade_progress_raw = match mem.read_i32(add_addr(
            agency_ptr,
            profile::DOMAIN53_AGENCY_UPGRADE_PROGRESS,
        )?) {
            Ok(value) if (-MAX_FIXED_RAW..=MAX_FIXED_RAW).contains(&value) => value,
            _ => {
                acc.skip();
                continue;
            }
        };
        let name_hash = match read_msvc_string(
            mem,
            add_addr(agency_ptr, profile::DOMAIN53_AGENCY_NAME)?,
            128,
        ) {
            StringRead::Value(value) => crate::compact::stable_hash64(&value),
            StringRead::Invalid => {
                acc.skip();
                0
            }
            StringRead::Missing => 0,
        };
        let icon_hash = match read_msvc_string(
            mem,
            add_addr(agency_ptr, profile::DOMAIN53_AGENCY_ICON)?,
            128,
        ) {
            StringRead::Value(value) => crate::compact::stable_hash64(&value),
            StringRead::Invalid => {
                acc.skip();
                0
            }
            StringRead::Missing => 0,
        };
        // Legacy wire fields carry raw capacity candidates only. 870ebf proof
        // rejects the old E4=max/F0=usable assignment; the authoritative raw
        // row below preserves E4/F0/F4 without semantic labels.
        let max_operatives =
            match mem.read_u32(add_addr(agency_ptr, profile::DOMAIN53_AGENCY_CAPACITY_E4)?) {
                Ok(value) if value <= MAX_AGENCY_CAPACITY_CLUSTER_VALUE => value,
                _ => {
                    acc.skip();
                    continue;
                }
            };
        let usable_slots =
            match mem.read_u32(add_addr(agency_ptr, profile::DOMAIN53_AGENCY_CAPACITY_F0)?) {
                Ok(value) if value <= MAX_AGENCY_CAPACITY_CLUSTER_VALUE => value,
                _ => {
                    acc.skip();
                    continue;
                }
            };
        acc.rows.agencies.push(AgencyRow {
            owner_tag_db_id: country.tag_db_id,
            max_operatives,
            usable_slots,
            operative_count: 0,
            cryptology_target_count: 0,
            defense_raw,
            upgrade_progress_raw,
            name_hash,
            icon_hash,
        });
        acc.rows.state_events.push(state_event(
            country.tag_db_id,
            None,
            STATE_EVENT_SOURCE_AGENCY_SCALAR,
            "agency_upgrade_progress_raw",
            &format!("agency_upgrade_progress_{upgrade_progress_raw}"),
            i64::from(upgrade_progress_raw),
        ));
        acc.observe();

        read_agency_capacity(mem, country.tag_db_id, agency_ptr, acc)?;
        agency_roots.push(AgencyRootRead {
            owner_tag_db_id: country.tag_db_id,
            agency_ptr,
        });
    }
    Ok(agency_roots)
}

fn read_agency_capacity<M: Domain53Memory>(
    mem: &M,
    owner_tag_db_id: u32,
    agency_ptr: u64,
    acc: &mut RowAccumulator,
) -> Result<(), ExtractError> {
    let value_0xe4 = mem.read_u32(add_addr(agency_ptr, profile::DOMAIN53_AGENCY_CAPACITY_E4)?)?;
    let value_0xf0 = mem.read_u32(add_addr(agency_ptr, profile::DOMAIN53_AGENCY_CAPACITY_F0)?)?;
    let value_0xf4 = mem.read_u32(add_addr(agency_ptr, profile::DOMAIN53_AGENCY_CAPACITY_F4)?)?;
    if value_0xe4 != value_0xf4
        || value_0xe4 > MAX_AGENCY_CAPACITY_CLUSTER_VALUE
        || value_0xf0 > MAX_AGENCY_CAPACITY_CLUSTER_VALUE
        || value_0xf4 > MAX_AGENCY_CAPACITY_CLUSTER_VALUE
    {
        acc.skip();
        return Ok(());
    }
    let proof_hash = crate::compact::stable_hash32("agency_capacity_offsets_e4_f0_f4");
    acc.rows
        .agency_capacity_clusters
        .push(AgencyCapacityClusterRow {
            owner_tag_db_id,
            value_0xe4,
            value_0xf0,
            value_0xf4,
            proof_hash,
        });
    acc.rows.state_events.push(state_event(
        owner_tag_db_id,
        None,
        STATE_EVENT_SOURCE_AGENCY_CAPACITY,
        "agency_capacity_offsets_e4_f0_f4",
        &format!("agency_capacity_{value_0xe4}_{value_0xf0}_{value_0xf4}"),
        i64::from(value_0xf0),
    ));
    acc.observe();
    Ok(())
}

fn read_cryptology_rows<M: Domain53Memory>(
    mem: &M,
    module_base: u64,
    agency_roots: &[AgencyRootRead],
    acc: &mut RowAccumulator,
) -> Result<(), ExtractError> {
    let crypto_vtable = add_addr(module_base, profile::DOMAIN53_CRYPTOLOGY_VTABLE_RVA)?;
    let record_vtable = add_addr(module_base, profile::DOMAIN53_CRYPTOLOGY_RECORD_VTABLE_RVA)?;
    let valid_owners = agency_roots
        .iter()
        .map(|root| root.owner_tag_db_id)
        .collect::<BTreeSet<_>>();
    let mut seen = BTreeSet::new();
    for agency in agency_roots {
        let crypto_ptr = match mem.read_u64(add_addr(
            agency.agency_ptr,
            profile::DOMAIN53_AGENCY_CRYPTO_MGR,
        )?) {
            Ok(0) => continue,
            Ok(value) if is_heap_ptr(value) => value,
            Ok(_) | Err(_) => {
                acc.skip();
                continue;
            }
        };
        let Ok(vtable) = mem.read_u64(crypto_ptr) else {
            acc.skip();
            continue;
        };
        if vtable != crypto_vtable {
            acc.skip();
            continue;
        }
        let data = match mem.read_u64(add_addr(crypto_ptr, profile::DOMAIN53_CRYPTOLOGY_MGR_DATA)?)
        {
            Ok(value) if value == 0 || is_heap_ptr(value) => value,
            _ => {
                acc.skip();
                continue;
            }
        };
        let size = match mem.read_u32(add_addr(crypto_ptr, profile::DOMAIN53_CRYPTOLOGY_MGR_SIZE)?)
        {
            Ok(value) if value <= profile::DOMAIN53_MAX_VECTOR_ROWS => value,
            _ => {
                acc.skip();
                continue;
            }
        };
        let cap = match mem.read_u32(add_addr(crypto_ptr, profile::DOMAIN53_CRYPTOLOGY_MGR_CAP)?) {
            Ok(value) if value >= size && value <= profile::DOMAIN53_MAX_VECTOR_ROWS => value,
            _ => {
                acc.skip();
                continue;
            }
        };
        let _ = cap;
        if size == 0 {
            continue;
        }
        if !is_heap_ptr(data) {
            acc.skip();
            continue;
        }
        for index in 0..size {
            let row_addr = data
                .checked_add(u64::from(index) * profile::DOMAIN53_CRYPTOLOGY_RECORD_STRIDE as u64)
                .ok_or(ExtractError::InvalidRoot(
                    "domain53_cryptology_record_overflow",
                ))?;
            let read = (|| -> Result<CryptologyRow, ExtractError> {
                if mem.read_u64(row_addr)? != record_vtable {
                    return Err(ExtractError::InvalidRoot(
                        "domain53_cryptology_vtable_mismatch",
                    ));
                }
                let target = mem.read_u32(add_addr(
                    row_addr,
                    profile::DOMAIN53_CRYPTOLOGY_RECORD_TARGET,
                )?)?;
                if !valid_tag_db_id(target) || !valid_owners.contains(&agency.owner_tag_db_id) {
                    return Err(ExtractError::InvalidRoot(
                        "domain53_cryptology_target_invalid",
                    ));
                }
                let decrypting = mem
                    .read_u8(add_addr(
                        row_addr,
                        profile::DOMAIN53_CRYPTOLOGY_RECORD_DECRYPTING,
                    )?)
                    .map(|value| value != 0)?;
                let amount_raw = mem.read_i64(add_addr(
                    row_addr,
                    profile::DOMAIN53_CRYPTOLOGY_RECORD_AMOUNT,
                )?)?;
                if !(0..=MAX_CRYPTO_AMOUNT_RAW).contains(&amount_raw) {
                    return Err(ExtractError::InvalidRoot(
                        "domain53_cryptology_amount_invalid",
                    ));
                }
                Ok(CryptologyRow {
                    owner_tag_db_id: agency.owner_tag_db_id,
                    target_tag_db_id: target,
                    amount_raw,
                    decrypting,
                })
            })();
            match read {
                Ok(row) if seen.insert((row.owner_tag_db_id, row.target_tag_db_id)) => {
                    acc.rows.state_events.push(state_event(
                        row.owner_tag_db_id,
                        Some(row.target_tag_db_id),
                        STATE_EVENT_SOURCE_CRYPTOLOGY,
                        "cryptology_amount_raw",
                        &format!("cryptology_amount_{}", row.amount_raw),
                        row.amount_raw,
                    ));
                    acc.rows.cryptology.push(row);
                    acc.observe();
                }
                Ok(_) => {}
                Err(_) => acc.skip(),
            }
        }
    }
    Ok(())
}

/// Wave-5 running operations: consume the COperationInstance heap hits from
/// the background discovery, revalidate each fresh, and emit
/// {instance_id, def_ref_ok, associated_country} rows.
///
/// Fail-closed per instance (the hit is DROPPED, counted as stale, on any
/// failure): the cached address must still carry the COperationInstance
/// vtable; the handle @+0x08 type (LO32) MUST == 4713 (else drop); the
/// operation-def ptr @+0x48 is vtable-checked as COperation to set
/// `def_ref_ok`; the associated-country ptr @+0x50 is vtable-checked as
/// CCountry and its tag_db_id read at +0x08 (a valid, in-range tag; else the
/// country is absent, NOT zero). Duplicate instance ids are dropped. The
/// +0x10 owner field is REJECTED and never read.
///
/// Returns (rows, stale_dropped). The caller folds stale_dropped into the
/// shared d53 staleness gate so a stale-majority degrades the whole frame.
/// How the operation-instance candidate hits split when re-read this frame.
///
/// The frame_degraded gate must reflect genuine object DEATH, not the discovery
/// needle's routine noise. `read_operations` therefore separates:
///   * `stale_liveness` — the cached address no longer holds the
///     COperationInstance vtable, or its handle qword can't be read (freed /
///     torn). This is the ONLY count that means "the cached set decayed."
///   * `filtered` — the address holds the vtable but is not a unique valid live
///     instance: wrong handle_type, out-of-range id, or a duplicate id already
///     emitted. These are vtable-value false positives / de-duplication, are
///     EXPECTED every frame, and previously (mis)counted as staleness — which is
///     what tripped the whole-frame gate and nuked all of d53 (see
///     runs/perf_rearch_20260706/D53_ROOT_CAUSE.md). Matches how `read_operatives`
///     already treats liveness (vtable) vs per-row validation (skip).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OperationDropBreakdown {
    pub stale_liveness: usize,
    pub filtered: usize,
}

fn read_operations<M: Domain53Memory>(
    mem: &M,
    module_base: u64,
    countries: &[CountryRoot],
    hits: &[u64],
) -> Result<(Vec<OperationInstanceRow>, OperationDropBreakdown), ExtractError> {
    let instance_vtable = add_addr(module_base, profile::DOMAIN53_OPERATION_INSTANCE_VTABLE_RVA)?;
    let def_vtable = add_addr(module_base, profile::DOMAIN53_OPERATION_DEF_VTABLE_RVA)?;
    let country_vtable = add_addr(module_base, profile::DOMAIN53_COUNTRY_VTABLE_RVA)?;
    let valid_countries = countries
        .iter()
        .map(|country| country.tag_db_id)
        .collect::<BTreeSet<_>>();
    let mut rows = Vec::new();
    let mut seen = BTreeSet::new();
    let mut drops = OperationDropBreakdown::default();
    // T3 sweep: resolve the candidate-loop prefetch distance ONCE (cached env
    // read; NOT per-candidate). `0` == prefetch OFF (block skipped). Default 32.
    let prefetch_dist_candidates = prefetch_dist_candidates();
    for (hit_index, hit) in hits.iter().enumerate() {
        let hit = *hit;
        // T3 (perf_rearch_20260706): prefetch-ahead (MLP). Same pattern as
        // `read_operatives`: while validating `hits[i]` (`read_u64(hit)` at the
        // object head), prefetch the candidate `prefetch_dist_candidates` ahead so
        // its DRAM latency overlaps. Raw u64 candidates; `prefetch_read` applies
        // the `is_heap_ptr` gate before the non-faulting hint; no guarded read
        // added; byte-identical. When the runtime distance is 0 (prefetch OFF arm
        // of the sweep), skip the block entirely — no `prefetch_read` is issued.
        if prefetch_dist_candidates != 0 {
            // `checked_add` guards against a pathological runtime distance
            // overflowing the index; on overflow we skip the lookahead (None),
            // same as an out-of-range `.get`.
            if let Some(&future_hit) = hit_index
                .checked_add(prefetch_dist_candidates)
                .and_then(|i| hits.get(i))
            {
                prefetch_read(future_hit);
            }
        }
        // Revalidate the cached vtable fresh. A vtable mismatch is genuine
        // staleness (the object at this address is gone / reused).
        match mem.read_u64(hit) {
            Ok(vtable) if vtable == instance_vtable => {}
            Ok(_) | Err(_) => {
                drops.stale_liveness = drops.stale_liveness.saturating_add(1);
                continue;
            }
        }
        // Handle @+0x08: id = HI32, type = LO32 (MUST == 4713). A handle we
        // cannot read on an address whose vtable just matched is a torn / mid-free
        // object -> genuine liveness failure.
        let Ok(handle) = mem.read_u64(add_addr(hit, profile::DOMAIN53_OPERATION_INSTANCE_HANDLE)?)
        else {
            drops.stale_liveness = drops.stale_liveness.saturating_add(1);
            continue;
        };
        // Wrong type tag == the vtable value was found in memory that is not a
        // real COperationInstance (needle false positive), NOT object death.
        let handle_type = (handle & 0xFFFF_FFFF) as u32;
        if handle_type != profile::DOMAIN53_OPERATION_INSTANCE_HANDLE_TYPE {
            drops.filtered = drops.filtered.saturating_add(1);
            continue;
        }
        let instance_id = (handle >> 32) as u32;
        if instance_id == 0 || instance_id > MAX_OPERATION_ID {
            drops.filtered = drops.filtered.saturating_add(1);
            continue;
        }
        if !seen.insert(instance_id) {
            drops.filtered = drops.filtered.saturating_add(1);
            continue;
        }
        // Def ptr @+0x48: vtable-check as COperation.
        let def_ref_ok =
            match mem.read_u64(add_addr(hit, profile::DOMAIN53_OPERATION_INSTANCE_DEF)?) {
                Ok(def_ptr) if is_heap_ptr(def_ptr) => {
                    matches!(mem.read_u64(def_ptr), Ok(vt) if vt == def_vtable)
                }
                _ => false,
            };
        // Associated country @+0x50: vtable-check as CCountry, read tag at +0x08.
        // PROVISIONAL (associated-country; target-vs-location undistinguished).
        let associated_country_tag_db_id = read_operation_associated_country(
            mem,
            add_addr(hit, profile::DOMAIN53_OPERATION_INSTANCE_TARGET_COUNTRY)?,
            country_vtable,
            &valid_countries,
        );
        rows.push(OperationInstanceRow {
            instance_id,
            def_ref_ok,
            associated_country_tag_db_id,
        });
    }
    rows.sort_by_key(|row| row.instance_id);
    Ok((rows, drops))
}

/// Resolve a COperationInstance's associated-country pointer (@+0x50) to a
/// tag_db_id. Returns `None` (absent, not zero) when the pointer is
/// null/invalid, fails the CCountry vtable check, or its tag is out of the
/// known-country set.
fn read_operation_associated_country<M: Domain53Memory>(
    mem: &M,
    country_ptr_addr: u64,
    country_vtable: u64,
    valid_countries: &BTreeSet<u32>,
) -> Option<u32> {
    let country_ptr = mem.read_u64(country_ptr_addr).ok()?;
    if !is_heap_ptr(country_ptr) {
        return None;
    }
    if mem.read_u64(country_ptr).ok()? != country_vtable {
        return None;
    }
    let tag_db_id = mem
        .read_u32(country_ptr.checked_add(profile::DOMAIN53_OBJ_TAG_DB_ID as u64)?)
        .ok()?;
    if !valid_tag_db_id(tag_db_id) || !valid_countries.contains(&tag_db_id) {
        return None;
    }
    Some(tag_db_id)
}

fn read_intel_network_graph_rows<M: Domain53Memory>(
    mem: &M,
    module_base: u64,
    countries: &[CountryRoot],
    strategic_operative_hits: &[u64],
    country_intel_network_hits: &[u64],
) -> Result<Option<Vec<IntelNetworkGraphRow>>, ExtractError> {
    if strategic_operative_hits.is_empty() || country_intel_network_hits.is_empty() {
        return Ok(None);
    }
    let strategic_vtable = add_addr(
        module_base,
        profile::DOMAIN53_STRATEGIC_OPERATIVE_VTABLE_RVA,
    )?;
    let network_vtable = add_addr(
        module_base,
        profile::DOMAIN53_COUNTRY_INTEL_NETWORK_VTABLE_RVA,
    )?;
    let valid_tags = countries
        .iter()
        .map(|country| country.tag_db_id)
        .collect::<BTreeSet<_>>();

    let mut networks = BTreeMap::new();
    for &network in country_intel_network_hits {
        match mem.read_u64(network) {
            Ok(vtable) if vtable == network_vtable => {}
            _ => return Ok(None),
        }
        let Ok(target) = mem.read_i32(add_addr(
            network,
            profile::DOMAIN53_COUNTRY_INTEL_NETWORK_TARGET,
        )?) else {
            return Ok(None);
        };
        if target <= 0 {
            return Ok(None);
        }
        let target = target as u32;
        if !valid_tags.contains(&target) {
            return Ok(None);
        }
        let Ok(graph) = mem.read_u64(add_addr(
            network,
            profile::DOMAIN53_COUNTRY_INTEL_NETWORK_GRAPH,
        )?) else {
            return Ok(None);
        };
        if !is_heap_ptr(graph) {
            return Ok(None);
        }
        let Ok(vertex_data) = mem.read_u64(add_addr(
            graph,
            profile::DOMAIN53_INTEL_NETWORK_GRAPH_VERTEX_DATA,
        )?) else {
            return Ok(None);
        };
        let Ok(vertex_count) = mem.read_u32(add_addr(
            graph,
            profile::DOMAIN53_INTEL_NETWORK_GRAPH_VERTEX_COUNT,
        )?) else {
            return Ok(None);
        };
        if vertex_count == 0 || vertex_count > profile::DOMAIN53_MAX_INTEL_NETWORK_STATES {
            return Ok(None);
        }
        if !is_heap_ptr(vertex_data) {
            return Ok(None);
        }
        let mut state_ids = Vec::with_capacity(vertex_count as usize);
        for index in 0..vertex_count {
            let vertex = add_addr(
                vertex_data,
                (index as usize)
                    .saturating_mul(profile::DOMAIN53_INTEL_NETWORK_GRAPH_VERTEX_STRIDE),
            )?;
            let Ok(state_id) = mem.read_i32(add_addr(
                vertex,
                profile::DOMAIN53_INTEL_NETWORK_GRAPH_VERTEX_STATE_ID,
            )?) else {
                return Ok(None);
            };
            if state_id <= 0 || state_id as u32 > MAX_INTEL_NETWORK_STATE_ID {
                return Ok(None);
            }
            state_ids.push(state_id as u32);
        }
        state_ids.sort_unstable();
        if state_ids.windows(2).any(|pair| pair[0] == pair[1]) {
            return Ok(None);
        }
        if networks.insert(network, (target, state_ids)).is_some() {
            return Ok(None);
        }
    }

    let mut reached = BTreeSet::new();
    let mut rows = Vec::new();
    let mut seen_slots = BTreeSet::new();
    for &operative in strategic_operative_hits {
        match mem.read_u64(operative) {
            Ok(vtable) if vtable == strategic_vtable => {}
            _ => return Ok(None),
        }
        let Ok(operative_index) = mem.read_u64(add_addr(
            operative,
            profile::DOMAIN53_STRATEGIC_OPERATIVE_INDEX,
        )?) else {
            return Ok(None);
        };
        if operative_index > u64::from(u32::MAX) {
            return Ok(None);
        }
        let operative_index = operative_index as u32;
        let Ok(data) = mem.read_u64(add_addr(
            operative,
            profile::DOMAIN53_STRATEGIC_OPERATIVE_NETWORK_VECTOR,
        )?) else {
            return Ok(None);
        };
        let Ok(size) = mem.read_u32(add_addr(
            operative,
            profile::DOMAIN53_STRATEGIC_OPERATIVE_NETWORK_SIZE,
        )?) else {
            return Ok(None);
        };
        let Ok(capacity) = mem.read_u32(add_addr(
            operative,
            profile::DOMAIN53_STRATEGIC_OPERATIVE_NETWORK_CAPACITY,
        )?) else {
            return Ok(None);
        };
        if size == 0 {
            continue;
        }
        if size > capacity || size > profile::DOMAIN53_MAX_INTEL_NETWORKS || !is_heap_ptr(data) {
            return Ok(None);
        }
        for slot in 0..size {
            let Ok(network) = mem.read_u64(add_addr(data, (slot as usize).saturating_mul(8))?)
            else {
                return Ok(None);
            };
            let Some((target, state_ids)) = networks.get(&network) else {
                return Ok(None);
            };
            if !reached.insert(network) {
                return Ok(None);
            }
            if !seen_slots.insert((operative_index, slot)) {
                return Ok(None);
            }
            rows.push(IntelNetworkGraphRow {
                strategic_operative_index: operative_index,
                network_slot: slot,
                target_tag_db_id: *target,
                state_ids: state_ids.clone(),
            });
        }
    }

    if rows.is_empty() || reached.len() != networks.len() {
        return Ok(None);
    }
    if networks.keys().any(|network| !reached.contains(network)) {
        return Ok(None);
    }
    rows.sort_by_key(|row| {
        (
            row.strategic_operative_index,
            row.network_slot,
            row.target_tag_db_id,
        )
    });
    Ok(Some(rows))
}

fn read_operatives<M: Domain53Memory>(
    mem: &M,
    module_base: u64,
    countries: &[CountryRoot],
    hits: &[u64],
    acc: &mut RowAccumulator,
) -> Result<usize, ExtractError> {
    let valid_owners = countries
        .iter()
        .map(|country| country.tag_db_id)
        .collect::<BTreeSet<_>>();
    let operative_vtable = add_addr(module_base, profile::DOMAIN53_OPERATIVE_VTABLE_RVA)?;
    let mut seen = BTreeSet::new();
    let mut stale_dropped = 0usize;
    // T3 sweep: resolve the candidate-loop prefetch distance ONCE (cached env
    // read; NOT per-candidate). `0` == prefetch OFF (block skipped). Default 32.
    let prefetch_dist_candidates = prefetch_dist_candidates();
    for (hit_index, hit) in hits.iter().enumerate() {
        let hit = *hit;
        // T3 (perf_rearch_20260706): prefetch-ahead (MLP). While validating this
        // candidate (`read_u64(hit)` at the object head), prefetch the candidate
        // `prefetch_dist_candidates` positions ahead so its DRAM latency overlaps.
        // `hits[i]` are raw u64 candidate addresses; `prefetch_read` applies the
        // `is_heap_ptr` gate (the same heap-range predicate the loop's reads rely
        // on) before the non-faulting hint. No guarded read is added — the
        // address is only prefetched, never dereferenced here — so rows/errors
        // are byte-identical. When the runtime distance is 0 (prefetch OFF arm of
        // the sweep), skip the block entirely — no `prefetch_read` is issued.
        if prefetch_dist_candidates != 0 {
            // `checked_add` guards against a pathological runtime distance
            // overflowing the index; on overflow we skip the lookahead (None),
            // same as an out-of-range `.get`.
            if let Some(&future_hit) = hit_index
                .checked_add(prefetch_dist_candidates)
                .and_then(|i| hits.get(i))
            {
                prefetch_read(future_hit);
            }
        }
        match mem.read_u64(hit) {
            Ok(vtable) if vtable == operative_vtable => {}
            Ok(_) | Err(_) => {
                stale_dropped = stale_dropped.saturating_add(1);
                continue;
            }
        }
        let identity = hit
            .checked_add(profile::DOMAIN53_OPERATIVE_IDENTITY_FROM_VTABLE as u64)
            .ok_or(ExtractError::InvalidRoot(
                "domain53_operative_identity_overflow",
            ))?;
        let read = read_operative_row(mem, identity, &valid_owners);
        match read {
            Ok(row) if seen.insert(row.op_id) => {
                if let Some(lifecycle) = row.lifecycle_flag {
                    acc.rows.state_events.push(state_event(
                        row.owner_tag_db_id,
                        None,
                        STATE_EVENT_SOURCE_OPERATIVE_LIFECYCLE,
                        &format!("operative_{}_lifecycle", row.op_id),
                        &format!("lifeflag_{lifecycle}"),
                        i64::from(lifecycle),
                    ));
                }
                acc.rows.operatives.push(row);
                acc.observe();
            }
            Ok(_) => {}
            Err(_) => acc.skip(),
        }
    }
    Ok(stale_dropped)
}

fn read_operative_row<M: Domain53Memory>(
    mem: &M,
    identity: u64,
    valid_owners: &BTreeSet<u32>,
) -> Result<OperativeRow, ExtractError> {
    let handle_type = mem.read_u32(identity)?;
    if handle_type != profile::DOMAIN53_OPERATIVE_HANDLE_TYPE {
        return Err(ExtractError::InvalidRoot("domain53_operative_handle_type"));
    }
    let op_id = mem.read_u32(add_addr(identity, 4)?)?;
    if op_id == 0 || op_id > MAX_OPERATIVE_ID {
        return Err(ExtractError::InvalidRoot("domain53_operative_id_invalid"));
    }
    let owner = mem.read_u32(add_addr(identity, profile::DOMAIN53_OPERATIVE_OWNER)?)?;
    if !valid_tag_db_id(owner) || !valid_owners.contains(&owner) {
        return Err(ExtractError::InvalidRoot(
            "domain53_operative_owner_invalid",
        ));
    }
    let lifecycle = mem.read_u32(add_addr(
        identity,
        profile::DOMAIN53_OPERATIVE_LIFECYCLE_FLAG,
    )?)?;
    if lifecycle > MAX_OPERATIVE_LIFECYCLE_FLAG {
        return Err(ExtractError::InvalidRoot(
            "domain53_operative_lifecycle_invalid",
        ));
    }
    let name_a = read_msvc_string(
        mem,
        add_addr(identity, profile::DOMAIN53_OPERATIVE_NAME)?,
        128,
    );
    let name_b = read_msvc_string(
        mem,
        add_addr(identity, profile::DOMAIN53_OPERATIVE_NAME_DUPLICATE)?,
        128,
    );
    let name_hash = match (name_a, name_b) {
        (StringRead::Value(a), StringRead::Value(b)) if a == b => crate::compact::stable_hash64(&a),
        (StringRead::Value(a), StringRead::Missing)
        | (StringRead::Missing, StringRead::Value(a)) => crate::compact::stable_hash64(&a),
        (StringRead::Missing, StringRead::Missing) => 0,
        (StringRead::Invalid, _) | (_, StringRead::Invalid) => {
            return Err(ExtractError::InvalidRoot(
                "domain53_operative_name_invalid_utf8",
            ));
        }
        _ => {
            return Err(ExtractError::InvalidRoot(
                "domain53_operative_name_mismatch",
            ))
        }
    };
    let detected = mem
        .read_u8(add_addr(identity, profile::DOMAIN53_OPERATIVE_DETECTED)?)
        .map(|value| value != 0)?;
    let leader = identity
        .checked_add_signed(profile::DOMAIN53_OPERATIVE_LEADER_FROM_IDENTITY_DELTA as i64)
        .ok_or(ExtractError::InvalidRoot(
            "domain53_operative_leader_overflow",
        ))?;
    let experience_raw = match mem.read_i64(add_addr(
        leader,
        profile::DOMAIN53_OPERATIVE_EXPERIENCE_RAW,
    )?) {
        Ok(value) if (0..=MAX_XP_RAW).contains(&value) => Some(value),
        Ok(_) => return Err(ExtractError::InvalidRoot("domain53_operative_xp_invalid")),
        Err(_) => None,
    };
    let skill = match mem.read_u64(add_addr(
        leader,
        profile::DOMAIN53_OPERATIVE_SKILL_SUBOBJECT,
    )?) {
        Ok(skill_obj) if is_heap_ptr(skill_obj) => {
            match mem.read_u32(add_addr(
                skill_obj,
                profile::DOMAIN53_OPERATIVE_SKILL_LEVEL,
            )?) {
                Ok(value) if value <= MAX_OPERATIVE_SKILL => Some(value),
                Ok(_) => {
                    return Err(ExtractError::InvalidRoot(
                        "domain53_operative_skill_invalid",
                    ))
                }
                Err(_) => None,
            }
        }
        _ => None,
    };
    Ok(OperativeRow {
        owner_tag_db_id: owner,
        op_id,
        detected,
        captured: false,
        skill,
        experience_raw,
        name_hash,
        state_hash: crate::compact::stable_hash32(&format!("lifeflag_{lifecycle}")),
        lifecycle_flag: Some(lifecycle),
    })
}

fn read_war_rows<M: Domain53Memory>(
    mem: &M,
    module_base: u64,
    countries: &[CountryRoot],
    war_hits: &[Vec<u64>; 2],
    acc: &mut RowAccumulator,
) -> Result<(Vec<WarRelationRow>, usize), ExtractError> {
    let valid_tags = countries
        .iter()
        .map(|country| country.tag_db_id)
        .collect::<BTreeSet<_>>();
    let date_vtables = domain53_war_date_vtables(module_base)?;
    let mut rows = Vec::new();
    let mut seen = BTreeSet::new();
    let mut stale_dropped = 0usize;
    for (date_vtable, hits) in date_vtables.iter().zip(war_hits.iter()) {
        for hit in hits {
            let hit = *hit;
            match mem.read_u64(hit) {
                Ok(vtable) if vtable == *date_vtable => {}
                Ok(_) | Err(_) => {
                    stale_dropped = stale_dropped.saturating_add(1);
                    continue;
                }
            }
            let Some(row_addr) = hit.checked_sub(profile::DOMAIN53_WAR_DATE_VTABLE as u64) else {
                continue;
            };
            let read = read_war_row(mem, row_addr, &valid_tags);
            match read {
                Ok(row) => {
                    let key = if row.first_tag_db_id <= row.second_tag_db_id {
                        (row.first_tag_db_id, row.second_tag_db_id)
                    } else {
                        (row.second_tag_db_id, row.first_tag_db_id)
                    };
                    if seen.insert(key) {
                        rows.push(row);
                        acc.observe();
                    }
                }
                Err(_) => acc.skip(),
            }
        }
    }
    Ok((rows, stale_dropped))
}

fn domain53_war_date_vtables(module_base: u64) -> Result<[u64; 2], ExtractError> {
    Ok([
        add_addr(module_base, profile::DOMAIN53_WAR_GREGORIAN_DATE_VTABLE_RVA)?,
        add_addr(module_base, profile::DOMAIN53_WAR_GAME_DATE_VTABLE_RVA)?,
    ])
}

/// P4b OFF-FRAME war-row address validation.
///
/// The war date-object vtables are extremely common heap objects — the
/// game-date vtable alone measured 1,456,331 live hits on D04 1944 — but only a
/// few dozen of those addresses are real war rows. `read_war_rows` validated ALL
/// of them EVERY FRAME on the frame thread (a ~0.3-1.5 s spike per frame, plus
/// it counted every non-war game-date object as a skipped row, which fails the
/// domain closed at 1944 scale). This function does that ~1.47M-hit filter ONCE
/// per sweep generation on a BACKGROUND thread, returning only the SMALL set of
/// validated war-row ADDRESSES. The per-frame path
/// (`read_war_rows_from_validated_addrs`) then re-reads the LIVE fields (tags,
/// casualties) fresh from those few addresses — values are never frozen.
///
/// Dedup is by unordered tag-pair, first-seen wins (gregorian hits before
/// game-date), byte-identical to `read_war_rows`' dedup so the emitted war-row
/// set is unchanged.
pub fn domain53_validate_war_row_addrs<M: Domain53Memory>(
    mem: &M,
    gregorian_hits: &[u64],
    game_date_hits: &[u64],
) -> Result<Vec<u64>, ExtractError> {
    let roots = resolve_domain53_frame_roots(mem)?;
    let module_base = roots.module_base;
    let valid_tags = roots
        .countries
        .iter()
        .map(|country| country.tag_db_id)
        .collect::<BTreeSet<_>>();
    let date_vtables = domain53_war_date_vtables(module_base)?;
    let hit_sources = [
        (date_vtables[0], gregorian_hits),
        (date_vtables[1], game_date_hits),
    ];
    let mut addrs = Vec::new();
    let mut seen = BTreeSet::new();
    for (date_vtable, hits) in hit_sources {
        for &hit in hits {
            match mem.read_u64(hit) {
                Ok(vtable) if vtable == date_vtable => {}
                Ok(_) | Err(_) => continue,
            }
            let Some(row_addr) = hit.checked_sub(profile::DOMAIN53_WAR_DATE_VTABLE as u64) else {
                continue;
            };
            let Ok(row) = read_war_row(mem, row_addr, &valid_tags) else {
                continue;
            };
            let key = if row.first_tag_db_id <= row.second_tag_db_id {
                (row.first_tag_db_id, row.second_tag_db_id)
            } else {
                (row.second_tag_db_id, row.first_tag_db_id)
            };
            if seen.insert(key) {
                addrs.push(row_addr);
            }
        }
    }
    Ok(addrs)
}

/// P4b PER-FRAME re-read of the validated war-row addresses produced off-frame
/// by [`domain53_validate_war_row_addrs`]. Re-reads the LIVE tags + casualties
/// from each address every frame (casualties change over time and must stay
/// fresh — never frozen). A validated address whose war-date vtable is gone
/// (freed/reused) is `stale_dropped`; one that still has the vtable but no
/// longer reads as a valid war row is a skip. Dedup by tag-pair defends against
/// a stale address aliasing a live war. This runs over only the ~dozens of
/// validated addresses, so it is cheap on the frame path.
fn read_war_rows_from_validated_addrs<M: Domain53Memory>(
    mem: &M,
    module_base: u64,
    countries: &[CountryRoot],
    war_row_addrs: &[u64],
    acc: &mut RowAccumulator,
) -> Result<(Vec<WarRelationRow>, usize), ExtractError> {
    let valid_tags = countries
        .iter()
        .map(|country| country.tag_db_id)
        .collect::<BTreeSet<_>>();
    let date_vtables = domain53_war_date_vtables(module_base)?;
    let mut rows = Vec::new();
    let mut seen = BTreeSet::new();
    let mut stale_dropped = 0usize;
    // T3 sweep: resolve the candidate-loop prefetch distance ONCE (cached env
    // read; NOT per-row). `0` == prefetch OFF (block skipped). Default 32.
    let prefetch_dist_candidates = prefetch_dist_candidates();
    for (row_index, &row_addr) in war_row_addrs.iter().enumerate() {
        // T3 (perf_rearch_20260706): prefetch-ahead (MLP). `war_row_addrs` is a
        // flat slice of raw u64 war-row base addresses (same shape as the
        // operative/operation candidate lists), so the same pattern applies:
        // while validating this row, prefetch the row-base `prefetch_dist_candidates`
        // ahead. The row's fields span offsets 0x00..=0x6C (two cache lines from
        // the base); prefetching the base line (which the first `read_u64` at
        // +0x08 touches) lets the hardware streamer follow to the casualty/echo
        // line. `prefetch_read` applies the `is_heap_ptr` gate before the
        // non-faulting hint; no guarded read added; byte-identical. When the
        // runtime distance is 0 (prefetch OFF arm of the sweep), skip the block
        // entirely — no `prefetch_read` is issued.
        if prefetch_dist_candidates != 0 {
            // `checked_add` guards against a pathological runtime distance
            // overflowing the index; on overflow we skip the lookahead (None),
            // same as an out-of-range `.get`.
            if let Some(&future_addr) = row_index
                .checked_add(prefetch_dist_candidates)
                .and_then(|i| war_row_addrs.get(i))
            {
                prefetch_read(future_addr);
            }
        }
        // Re-validate the date-object vtable at this address (ABA defense: a
        // freed/reused slot no longer carries a war date vtable → stale).
        let Some(vtable_addr) = row_addr.checked_add(profile::DOMAIN53_WAR_DATE_VTABLE as u64)
        else {
            stale_dropped = stale_dropped.saturating_add(1);
            continue;
        };
        match mem.read_u64(vtable_addr) {
            Ok(vtable) if date_vtables.contains(&vtable) => {}
            Ok(_) | Err(_) => {
                stale_dropped = stale_dropped.saturating_add(1);
                continue;
            }
        }
        match read_war_row(mem, row_addr, &valid_tags) {
            Ok(row) => {
                let key = if row.first_tag_db_id <= row.second_tag_db_id {
                    (row.first_tag_db_id, row.second_tag_db_id)
                } else {
                    (row.second_tag_db_id, row.first_tag_db_id)
                };
                if seen.insert(key) {
                    rows.push(row);
                    acc.observe();
                }
            }
            Err(_) => acc.skip(),
        }
    }
    Ok((rows, stale_dropped))
}

fn read_war_row<M: Domain53Memory>(
    mem: &M,
    row_addr: u64,
    valid_tags: &BTreeSet<u32>,
) -> Result<WarRelationRow, ExtractError> {
    let first = mem.read_u32(add_addr(row_addr, profile::DOMAIN53_WAR_FIRST_TAG)?)?;
    let second = mem.read_u32(add_addr(row_addr, profile::DOMAIN53_WAR_SECOND_TAG)?)?;
    if first == second
        || !valid_tag_db_id(first)
        || !valid_tag_db_id(second)
        || !valid_tags.contains(&first)
        || !valid_tags.contains(&second)
    {
        return Err(ExtractError::InvalidRoot("domain53_war_pair_invalid"));
    }
    let first_casualties =
        mem.read_i32(add_addr(row_addr, profile::DOMAIN53_WAR_FIRST_CASUALTIES)?)? as i64;
    let second_casualties =
        mem.read_i32(add_addr(row_addr, profile::DOMAIN53_WAR_SECOND_CASUALTIES)?)? as i64;
    if !(0..=MAX_CASUALTIES).contains(&first_casualties)
        || !(0..=MAX_CASUALTIES).contains(&second_casualties)
    {
        return Err(ExtractError::InvalidRoot("domain53_war_casualties_invalid"));
    }
    let echo_first = mem.read_u32(add_addr(row_addr, profile::DOMAIN53_WAR_ECHO_FIRST)?)?;
    let echo_second = mem.read_u32(add_addr(row_addr, profile::DOMAIN53_WAR_ECHO_SECOND)?)?;
    if echo_first != first || echo_second != second {
        return Err(ExtractError::InvalidRoot("domain53_war_echo_mismatch"));
    }
    Ok(WarRelationRow {
        first_tag_db_id: first,
        second_tag_db_id: second,
        first_casualties,
        second_casualties,
    })
}

fn war_projection_by_country(rows: &[WarRelationRow]) -> BTreeMap<u32, (u32, i64)> {
    let mut out = BTreeMap::<u32, (u32, i64)>::new();
    for row in rows {
        let first = out.entry(row.first_tag_db_id).or_default();
        first.0 = first.0.saturating_add(1);
        first.1 = first.1.saturating_add(row.first_casualties);
        let second = out.entry(row.second_tag_db_id).or_default();
        second.0 = second.0.saturating_add(1);
        second.1 = second.1.saturating_add(row.second_casualties);
    }
    out
}

fn relation_vtable_kinds(module_base: u64) -> Result<BTreeMap<u64, u16>, ExtractError> {
    Ok(BTreeMap::from([
        (
            add_addr(
                module_base,
                profile::DOMAIN53_REL_NON_AGGRESSION_PACT_VTABLE_RVA,
            )?,
            REL_KIND_NON_AGGRESSION_PACT,
        ),
        (
            add_addr(module_base, profile::DOMAIN53_REL_SUBJECT_VTABLE_RVA)?,
            REL_KIND_PUPPET,
        ),
        (
            add_addr(module_base, profile::DOMAIN53_REL_EMBARGO_VTABLE_RVA)?,
            REL_KIND_EMBARGO,
        ),
        (
            add_addr(
                module_base,
                profile::DOMAIN53_REL_MILITARY_ACCESS_VTABLE_RVA,
            )?,
            REL_KIND_MILITARY_ACCESS,
        ),
        (
            add_addr(
                module_base,
                profile::DOMAIN53_REL_AIR_BASE_ACCESS_VTABLE_RVA,
            )?,
            REL_KIND_AIR_BASE_ACCESS,
        ),
        (
            add_addr(module_base, profile::DOMAIN53_REL_DOCKING_RIGHTS_VTABLE_RVA)?,
            REL_KIND_DOCKING_RIGHTS,
        ),
        (
            add_addr(module_base, profile::DOMAIN53_REL_NAVAL_BLOCKADE_VTABLE_RVA)?,
            REL_KIND_NAVAL_BLOCKADE,
        ),
        (
            add_addr(
                module_base,
                profile::DOMAIN53_REL_MARKET_ACCESS_RIGHTS_VTABLE_RVA,
            )?,
            REL_KIND_MARKET_ACCESS_RIGHTS,
        ),
        (
            add_addr(
                module_base,
                profile::DOMAIN53_REL_EQUIPMENT_PURCHASE_RELATION_VTABLE_RVA,
            )?,
            REL_KIND_EQUIPMENT_PURCHASE_CONTRACT_RELATION,
        ),
        (
            add_addr(module_base, profile::DOMAIN53_REL_GUARANTEE_VTABLE_RVA)?,
            REL_KIND_GUARANTEE,
        ),
        (
            add_addr(
                module_base,
                profile::DOMAIN53_REL_IMPROVE_RELATIONS_VTABLE_RVA,
            )?,
            REL_KIND_IMPROVE_RELATIONS,
        ),
        (
            add_addr(module_base, profile::DOMAIN53_REL_ATTACHE_VTABLE_RVA)?,
            REL_KIND_ATTACHE,
        ),
        (
            add_addr(
                module_base,
                profile::DOMAIN53_REL_LICENCE_PRODUCTION_VTABLE_RVA,
            )?,
            REL_KIND_LICENCE_PRODUCTION,
        ),
    ]))
}

fn scan_qword_hits<M: Domain53Memory>(
    mem: &M,
    ranges: &[(u64, usize)],
    needle: u64,
) -> Result<Vec<u64>, ExtractError> {
    let needle_bytes = needle.to_le_bytes();
    let mut hits = Vec::new();
    const CHUNK: usize = 1 << 20;
    const OVERLAP: usize = 16;
    for (start, len) in ranges {
        let Some(end) = start.checked_add(*len as u64) else {
            continue;
        };
        let mut pos = *start;
        while pos < end {
            let remaining = (end - pos).min(usize::MAX as u64) as usize;
            let want = remaining.min(CHUNK);
            let blob = mem.read_bytes(pos, want)?;
            let mut offset = (8usize.wrapping_sub((pos as usize) % 8)) % 8;
            while offset + 8 <= blob.len() {
                if blob[offset..offset + 8] == needle_bytes {
                    hits.push(pos + offset as u64);
                }
                offset += 8;
            }
            if blob.len() < want || blob.len() >= remaining {
                break;
            }
            pos = pos
                .checked_add(blob.len().saturating_sub(OVERLAP).max(1) as u64)
                .ok_or(ExtractError::InvalidRoot("domain53_scan_overflow"))?;
        }
    }
    Ok(hits)
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum StringRead {
    Value(String),
    Missing,
    Invalid,
}

fn read_msvc_string<M: Domain53Memory>(mem: &M, address: u64, max_len: usize) -> StringRead {
    let Ok(size) = mem.read_u64(add_addr(address, 0x10).unwrap_or(address)) else {
        return StringRead::Missing;
    };
    let Ok(cap) = mem.read_u64(add_addr(address, 0x18).unwrap_or(address)) else {
        return StringRead::Missing;
    };
    let size = size as usize;
    let cap = cap as usize;
    if size == 0 {
        return StringRead::Missing;
    }
    if size > max_len || cap < size || cap > (1 << 20) {
        return StringRead::Invalid;
    }
    let chars_addr = if cap < 16 {
        address
    } else {
        match mem.read_u64(address) {
            Ok(ptr) if is_heap_ptr(ptr) => ptr,
            _ => return StringRead::Invalid,
        }
    };
    let Ok(bytes) = mem.read_bytes(chars_addr, size + 1) else {
        return StringRead::Missing;
    };
    if bytes.get(size).copied() != Some(0) {
        return StringRead::Invalid;
    }
    match std::str::from_utf8(&bytes[..size]) {
        Ok(value) if !value.chars().any(char::is_control) => StringRead::Value(value.to_string()),
        _ => StringRead::Invalid,
    }
}

fn state_event(
    owner_tag_db_id: u32,
    target_tag_db_id: Option<u32>,
    source_kind: u16,
    object: &str,
    value: &str,
    value_raw: i64,
) -> StateEventRow {
    StateEventRow {
        owner_tag_db_id,
        target_tag_db_id,
        event_kind: STATE_EVENT_KIND_OBSERVED,
        source_kind,
        object_hash: crate::compact::stable_hash32(object),
        value_hash: crate::compact::stable_hash32(value),
        value_raw,
    }
}

fn add_addr(base: u64, offset: usize) -> Result<u64, ExtractError> {
    base.checked_add(offset as u64)
        .ok_or(ExtractError::InvalidRoot("domain53_address_overflow"))
}

fn is_heap_ptr(value: u64) -> bool {
    value >= profile::DOMAIN53_HEAP_LO as u64 && value < profile::DOMAIN53_HEAP_HI as u64
}

/// T3 (perf_rearch_20260706): software-prefetch hint, **perf-only, zero
/// semantic effect**. For an `is_heap_ptr` `addr`, issues a `PREFETCHT0` (bring
/// the containing 64-byte line toward L1) to overlap DRAM latency with useful
/// work in the d53 scan loops; for any other `addr` it does nothing. It adds NO
/// guarded memory read: nothing is dereferenced here, the loops' own reads are
/// unchanged, and removing every call to this function leaves
/// rows/wire-bytes/errors byte-identical.
///
/// The `is_heap_ptr` gate is enforced HERE, on the exact address handed to the
/// intrinsic — not left to caller discipline. Callers may pass a base pointer
/// plus a field offset (the relation site passes `base + 0x300` / `base +
/// 0x288`); whatever the derived address, only one that itself satisfies
/// `is_heap_ptr` ever reaches `_mm_prefetch`.
///
/// SAFETY / faulting: `_mm_prefetch` (`PREFETCHh`) is architecturally
/// non-faulting for canonical addresses — it never raises `#PF` on
/// unmapped/invalid-but-canonical addresses (it is a hint the CPU may ignore).
/// The one 64-bit hazard is a *non-canonical* effective address, which can
/// `#GP`. `is_heap_ptr`'s window `[DOMAIN53_HEAP_LO, DOMAIN53_HEAP_HI) =
/// [0x100_0000_0000, 0x7FF0_0000_0000)` lies entirely in the canonical low
/// half (well below the non-canonical boundary `0x0000_8000_0000_0000`), so any
/// `addr` that passes the guard is canonical and cannot `#GP`. The pointer is
/// passed to the hint only; it is never read or written. See
/// `CODEX_D53_REDIAG_VERDICT.txt` C7.
#[inline(always)]
fn prefetch_read(addr: u64) {
    // Filter the EXACT address handed to the intrinsic (contract: prefetch only
    // `is_heap_ptr`-passing addresses). A base+offset derived address that lands
    // outside the heap window is simply not prefetched — harmless, since this is
    // a hint with no semantic effect.
    if !is_heap_ptr(addr) {
        return;
    }
    #[cfg(target_arch = "x86_64")]
    {
        use core::arch::x86_64::{_mm_prefetch, _MM_HINT_T0};
        // SAFETY: the `is_heap_ptr(addr)` guard above guarantees `addr` is inside
        // the heap window, which lies entirely in the canonical low half (see the
        // doc comment), so `addr` is canonical and `_mm_prefetch` cannot `#GP`
        // (and PREFETCHh never `#PF`s on canonical addresses). The address is
        // used only as a prefetch hint and is never dereferenced, so no memory is
        // read or written here.
        unsafe {
            _mm_prefetch::<{ _MM_HINT_T0 }>(addr as *const i8);
        }
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        // Prefetch is a perf-only hint with no portable equivalent; on other
        // architectures this is an intentional no-op (the address is not used).
        let _ = addr;
    }
}

fn in_module(module_base: u64, value: u64) -> bool {
    (module_base..module_base.saturating_add(DOMAIN53_MODULE_SPAN_MAX)).contains(&value)
}

fn valid_tag_db_id(value: u32) -> bool {
    value > 0 && value <= MAX_TAG_DB_ID
}

/// W1.2 instrument (perf_rearch_20260706): elapsed-micros breakdown of the
/// four sub-phases INSIDE [`build_domain53_payload_with_status`], splitting the
/// single collector `seg_payload_us` timer (W1.1 residual, ~39 ms/frame) so the
/// analysis knows WHICH sub-phase to optimize:
/// - `snapshot_us`: the `DiplomacyIntelSnapshot { .. }` construction (moving the
///   row Vecs into the snapshot).
/// - `validation_us`: the source-proof / `require_rows` validation block,
///   including its repeated O(rows) `state_events.iter().any(..)` full scans and
///   the status-flag adjustments.
/// - `encode_us`: `encode_payload(&snapshot)` — serialize all rows to wire bytes.
/// - `summary_us`: `summarize_snapshot(0, &snapshot)` — the W2 encode-side gate
///   summary that REPLACES the per-frame re-decode on the production hot path.
/// - `decode_gate_us`: `decode_payload(0, &payload)` — now runs ONLY on the 1/N
///   SAMPLED cross-check frames (0 on off-sample frames). Was the per-frame
///   re-parse of the ~380k-row payload before W2.
///
/// These spans run in the SAME order as the timed statements and their sum is
/// slightly LESS than the collector `seg_payload_us` (the small remainder = the
/// report struct assembly + the collector-side rows move / bookkeeping outside
/// this function). Observational only — the timing is not read by any non-perf
/// caller and never touches the payload bytes, gate summary, status flags, or
/// errors. It is neutralized in `PartialEq`/`Eq` (see the manual impl) so equality
/// of two reports is unaffected by measurement jitter.
#[derive(Clone, Copy, Debug, Default)]
pub struct PayloadPhaseTimes {
    pub snapshot_us: u64,
    pub validation_us: u64,
    pub encode_us: u64,
    pub summary_us: u64,
    pub decode_gate_us: u64,
}

#[derive(Clone, Debug)]
pub struct Domain53ExtractionReport {
    pub domain_id: u16,
    pub source_proofs: u64,
    pub snapshot: DiplomacyIntelSnapshot,
    pub payload: Vec<u8>,
    pub gate_summary: Domain53GateSummary,
    pub errors: Vec<String>,
    /// W1.2 observational sub-phase timing (see [`PayloadPhaseTimes`]). Excluded
    /// from `PartialEq`/`Eq` so wall-clock jitter never changes report equality.
    pub payload_phase_times: PayloadPhaseTimes,
}

// W1.2: `Domain53ExtractionReport` previously `#[derive(PartialEq, Eq)]`. The new
// `payload_phase_times` field is observational wall-clock jitter, so equality must
// IGNORE it (identical rows/proofs produce identical payload/gate/errors but not
// identical timing). Hand-write the impls over the semantic fields only — this is
// the T4/W1.1 "neutralize only the new observational field on both sides" pattern.
impl PartialEq for Domain53ExtractionReport {
    fn eq(&self, other: &Self) -> bool {
        self.domain_id == other.domain_id
            && self.source_proofs == other.source_proofs
            && self.snapshot == other.snapshot
            && self.payload == other.payload
            && self.gate_summary == other.gate_summary
            && self.errors == other.errors
    }
}
impl Eq for Domain53ExtractionReport {}

impl Domain53ExtractionReport {
    pub fn ok(&self) -> bool {
        self.errors.is_empty()
    }
}

/// W2 sampled-verify cadence: the production path derives the
/// [`Domain53GateSummary`] encode-side via [`summarize_snapshot`]; on 1 out of
/// every `DECODE_VERIFY_SAMPLE_N` frames it ALSO runs the full `decode_payload`
/// and asserts the two summaries are field-identical, failing closed on any
/// mismatch (so an encoder regression / heap tear that desyncs the bytes from the
/// snapshot is still caught, at 1/N the per-frame cost). N=32: at the ~73 ms
/// post-W2 d53 frame this amortizes the 29.7 ms decode to ~0.9 ms/frame (~1.3%)
/// while catching drift within ~32 frames.
const DECODE_VERIFY_SAMPLE_N: u64 = 32;

/// Monotonic frame counter for the sampled decode cross-check. Process-global and
/// deterministic (increments once per production build call). Tests force
/// sample-every-frame via the private `sample` param on
/// [`build_domain53_payload_sampled`], so this counter is never consulted in the
/// equality/fail-closed tests.
static DOMAIN53_BUILD_FRAME_COUNTER: AtomicU64 = AtomicU64::new(0);

/// True on 1/`DECODE_VERIFY_SAMPLE_N` calls. `fetch_add` gives each production
/// call a distinct index; index 0 (the first frame) samples so a totally-broken
/// encoder is caught on frame 1.
fn should_sample_decode_verify() -> bool {
    let index = DOMAIN53_BUILD_FRAME_COUNTER.fetch_add(1, Ordering::Relaxed);
    index % DECODE_VERIFY_SAMPLE_N == 0
}

/// On a sampled frame, compare the encode-side summary to the authoritative
/// decode. `None` when they agree (the expected case). `Some(error)` on ANY
/// difference — the error is pushed into `errors`, making `report.ok()` false so
/// the collector fails the frame closed to SOURCE_UNAVAILABLE, IDENTICAL to a
/// decode parse-error before W2. Factored out so the fail-closed wiring is unit
/// tested without needing to force a genuine encoder bug in the hot path.
fn decode_verify_mismatch_error(
    encode_summary: &Domain53GateSummary,
    decode_summary: &Domain53GateSummary,
) -> Option<String> {
    if encode_summary == decode_summary {
        None
    } else {
        Some(format!(
            "domain{DOMAIN_ID_PROPOSED} encode-side gate summary disagrees with sampled decode (fail-closed): encode={encode_summary:?} decode={decode_summary:?}"
        ))
    }
}

pub fn build_domain53_payload(
    rows: ExtractedDomain53Rows,
    source_proofs: u64,
) -> Domain53ExtractionReport {
    build_domain53_payload_with_status(rows, source_proofs, 0)
}

/// Like [`build_domain53_payload`] but seeds the payload snapshot's
/// `status_flags` with `extra_status_flags` (e.g. `STATUS_PARTIAL_DATA` for an
/// expected-partial frame that carries valid rows plus known per-row omissions).
/// The extra bits are ORed BEFORE encode so the host fold reads them from the
/// decoded payload status (not just the record header). All validation /
/// source-proof logic is unchanged; passing 0 is byte-identical to the 2-arg
/// form.
pub fn build_domain53_payload_with_status(
    rows: ExtractedDomain53Rows,
    source_proofs: u64,
    extra_status_flags: u32,
) -> Domain53ExtractionReport {
    let sample_decode = should_sample_decode_verify();
    build_domain53_payload_sampled(rows, source_proofs, extra_status_flags, sample_decode)
}

/// Inner builder with an EXPLICIT `sample_decode` flag (production computes it from
/// [`should_sample_decode_verify`]; tests pass `true` to force the decode
/// cross-check every call and exercise the fail-closed path deterministically).
///
/// W2: the production gate summary is produced ENCODE-SIDE by
/// [`summarize_snapshot`] (no ~380k-row re-decode). When `sample_decode` is true
/// the full `decode_payload` also runs and its summary is compared field-for-field
/// to the encode-side one; a mismatch pushes an error so `report.ok()` is false —
/// identical fail-closed semantics to a decode failure before W2.
fn build_domain53_payload_sampled(
    rows: ExtractedDomain53Rows,
    source_proofs: u64,
    extra_status_flags: u32,
    sample_decode: bool,
) -> Domain53ExtractionReport {
    // W1.2 instrument: split the single collector `seg_payload_us` timer into
    // disjoint sub-phases (snapshot / validation / encode / summary / decode-gate).
    // Purely observational — the timed statements run in the same order; the
    // returned payload/gate/errors do not depend on the timing.
    let snapshot_started = Instant::now();
    let mut snapshot = DiplomacyIntelSnapshot {
        elapsed_wall_ms: rows.elapsed_wall_ms,
        status_flags: extra_status_flags,
        changed_rows: rows.changed_rows,
        countries: rows.countries,
        wars: rows.wars,
        opinions: rows.opinions,
        relation_flags: rows.relation_flags,
        subjects: rows.subjects,
        agencies: rows.agencies,
        agency_capacity_clusters: rows.agency_capacity_clusters,
        operatives: rows.operatives,
        cryptology: rows.cryptology,
        operation_anchors: rows.operation_anchors,
        intel_map: rows.intel_map,
        state_events: rows.state_events,
        operation_instances: rows.operation_instances,
        incoming_actions: rows.incoming_actions,
        intel_network_graphs: rows.intel_network_graphs,
        factions: rows.factions,
    };
    let snapshot_us = duration_micros_u64_saturating(snapshot_started.elapsed());

    let validation_started = Instant::now();
    let mut errors = Vec::new();

    let unknown_source_bits = source_proofs & !KNOWN_SOURCE_PROOFS;
    if unknown_source_bits != 0 {
        errors.push(format!(
            "domain{DOMAIN_ID_PROPOSED} source_proofs has unknown bits 0x{unknown_source_bits:X}"
        ));
    }
    if snapshot.row_count() != 0 && !has(source_proofs, SOURCE_PROOF_BUILD_SHA_870EBF) {
        errors.push(format!(
            "domain{DOMAIN_ID_PROPOSED} rows require SOURCE_PROOF_BUILD_SHA_870EBF for {EXPECTED_HOI4_SHA256_870EBF}"
        ));
    }

    require_rows(
        !snapshot.countries.is_empty(),
        source_proofs,
        SOURCE_PROOF_COUNTRY_ROOTS_870EBF,
        "country diplomacy rows require SOURCE_PROOF_COUNTRY_ROOTS_870EBF",
        &mut errors,
    );
    require_rows(
        !snapshot.countries.is_empty(),
        source_proofs,
        SOURCE_PROOF_FACTION_GROUP_870EBF,
        "country diplomacy rows require SOURCE_PROOF_FACTION_GROUP_870EBF",
        &mut errors,
    );
    require_rows(
        !snapshot.opinions.is_empty()
            || !snapshot.relation_flags.is_empty()
            || !snapshot.subjects.is_empty(),
        source_proofs,
        SOURCE_PROOF_RELATION_ROOTS_870EBF,
        "opinion/relation/subject rows require SOURCE_PROOF_RELATION_ROOTS_870EBF",
        &mut errors,
    );
    require_rows(
        !snapshot.agencies.is_empty() || !snapshot.agency_capacity_clusters.is_empty(),
        source_proofs,
        SOURCE_PROOF_AGENCY_ROOT_870EBF,
        "agency rows/capacity clusters require SOURCE_PROOF_AGENCY_ROOT_870EBF",
        &mut errors,
    );
    require_rows(
        !snapshot.operatives.is_empty(),
        source_proofs,
        SOURCE_PROOF_OPERATIVE_ROOT_870EBF,
        "operative rows require SOURCE_PROOF_OPERATIVE_ROOT_870EBF",
        &mut errors,
    );
    require_rows(
        !snapshot.cryptology.is_empty(),
        source_proofs,
        SOURCE_PROOF_CRYPTOLOGY_ROOT_870EBF,
        "cryptology rows require SOURCE_PROOF_CRYPTOLOGY_ROOT_870EBF",
        &mut errors,
    );
    require_rows(
        !snapshot.incoming_actions.is_empty(),
        source_proofs,
        SOURCE_PROOF_INCOMING_ACTION_QUEUE_870EBF,
        "incoming diplomatic action rows require SOURCE_PROOF_INCOMING_ACTION_QUEUE_870EBF",
        &mut errors,
    );
    require_rows(
        !snapshot.intel_network_graphs.is_empty(),
        source_proofs,
        SOURCE_PROOF_INTEL_NETWORK_GRAPH_870EBF,
        "intel-network graph rows require SOURCE_PROOF_INTEL_NETWORK_GRAPH_870EBF",
        &mut errors,
    );
    require_rows(
        !snapshot.factions.is_empty(),
        source_proofs,
        SOURCE_PROOF_FACTION_SYSTEM_870EBF,
        "faction roster rows require SOURCE_PROOF_FACTION_SYSTEM_870EBF",
        &mut errors,
    );
    require_rows(
        snapshot.state_events.iter().any(|row| {
            matches!(
                row.source_kind,
                STATE_EVENT_SOURCE_RELATION_FLAG
                    | STATE_EVENT_SOURCE_RELATION_OPINION
                    | STATE_EVENT_SOURCE_SUBJECT_AUTONOMY
            )
        }),
        source_proofs,
        SOURCE_PROOF_RELATION_ROOTS_870EBF,
        "compact state events sourced from relation/opinion/subject rows require SOURCE_PROOF_RELATION_ROOTS_870EBF",
        &mut errors,
    );
    require_rows(
        snapshot
            .state_events
            .iter()
            .any(|row| row.source_kind == STATE_EVENT_SOURCE_COUNTRY_FACTION_GROUP),
        source_proofs,
        SOURCE_PROOF_COUNTRY_ROOTS_870EBF,
        "compact state events sourced from country faction rows require SOURCE_PROOF_COUNTRY_ROOTS_870EBF",
        &mut errors,
    );
    require_rows(
        snapshot
            .state_events
            .iter()
            .any(|row| row.source_kind == STATE_EVENT_SOURCE_COUNTRY_FACTION_GROUP),
        source_proofs,
        SOURCE_PROOF_FACTION_GROUP_870EBF,
        "compact state events sourced from country faction rows require SOURCE_PROOF_FACTION_GROUP_870EBF",
        &mut errors,
    );
    require_rows(
        snapshot.state_events.iter().any(|row| {
            matches!(
                row.source_kind,
                STATE_EVENT_SOURCE_AGENCY_SCALAR | STATE_EVENT_SOURCE_AGENCY_CAPACITY
            )
        }),
        source_proofs,
        SOURCE_PROOF_AGENCY_ROOT_870EBF,
        "compact state events sourced from agency rows require SOURCE_PROOF_AGENCY_ROOT_870EBF",
        &mut errors,
    );
    require_rows(
        snapshot
            .state_events
            .iter()
            .any(|row| row.source_kind == STATE_EVENT_SOURCE_OPERATIVE_LIFECYCLE),
        source_proofs,
        SOURCE_PROOF_OPERATIVE_ROOT_870EBF,
        "compact state events sourced from operative lifecycle rows require SOURCE_PROOF_OPERATIVE_ROOT_870EBF",
        &mut errors,
    );
    require_rows(
        snapshot
            .state_events
            .iter()
            .any(|row| row.source_kind == STATE_EVENT_SOURCE_CRYPTOLOGY),
        source_proofs,
        SOURCE_PROOF_CRYPTOLOGY_ROOT_870EBF,
        "compact state events sourced from cryptology rows require SOURCE_PROOF_CRYPTOLOGY_ROOT_870EBF",
        &mut errors,
    );

    let has_country_war_projection = snapshot
        .countries
        .iter()
        .any(|row| row.at_war_count != 0 || row.casualties != 0);
    if has(source_proofs, SOURCE_PROOF_WAR_ROOT_870EBF) {
        snapshot.status_flags &= !STATUS_WAR_ROOT_UNVALIDATED;
    } else {
        snapshot.status_flags |= STATUS_WAR_ROOT_UNVALIDATED;
        if !snapshot.wars.is_empty() {
            errors.push("war rows require SOURCE_PROOF_WAR_ROOT_870EBF".to_string());
        }
        if has_country_war_projection {
            errors.push(
                "country at-war/casualty projections require SOURCE_PROOF_WAR_ROOT_870EBF"
                    .to_string(),
            );
        }
    }
    if has(source_proofs, SOURCE_PROOF_WAR_LIFECYCLE_DETAIL_870EBF) {
        snapshot.status_flags &= !STATUS_WAR_LIFECYCLE_DETAIL_UNVALIDATED;
    } else {
        snapshot.status_flags |= STATUS_WAR_LIFECYCLE_DETAIL_UNVALIDATED;
    }

    let has_faction_rows = !snapshot.countries.is_empty()
        || snapshot
            .state_events
            .iter()
            .any(|row| row.source_kind == STATE_EVENT_SOURCE_COUNTRY_FACTION_GROUP);
    if has(source_proofs, SOURCE_PROOF_FACTION_GROUP_870EBF) {
        snapshot.status_flags &= !STATUS_FACTION_GROUP_UNVALIDATED;
    } else {
        snapshot.status_flags |= STATUS_FACTION_GROUP_UNVALIDATED;
        if has_faction_rows {
            errors
                .push("country faction rows require SOURCE_PROOF_FACTION_GROUP_870EBF".to_string());
        }
    }

    if has(source_proofs, SOURCE_PROOF_INCOMING_ACTION_QUEUE_870EBF) {
        snapshot.status_flags &= !STATUS_INCOMING_ACTION_QUEUE_UNVALIDATED;
    } else {
        snapshot.status_flags |= STATUS_INCOMING_ACTION_QUEUE_UNVALIDATED;
    }

    if has(source_proofs, SOURCE_PROOF_INTEL_NETWORK_GRAPH_870EBF) {
        snapshot.status_flags &= !STATUS_INTEL_NETWORK_GRAPH_UNVALIDATED;
    } else {
        snapshot.status_flags |= STATUS_INTEL_NETWORK_GRAPH_UNVALIDATED;
    }

    let has_resisted_operations = snapshot
        .operation_anchors
        .iter()
        .any(|row| row.detail_resisted);
    let has_unresisted_operations = snapshot
        .operation_anchors
        .iter()
        .any(|row| !row.detail_resisted);
    if has_resisted_operations || !has(source_proofs, SOURCE_PROOF_OPERATION_DETAIL_870EBF) {
        snapshot.status_flags |= STATUS_OPERATIONS_DETAIL_RESISTED;
    }
    require_rows(
        has_unresisted_operations,
        source_proofs,
        SOURCE_PROOF_OPERATION_DETAIL_870EBF,
        "unresisted operation rows require SOURCE_PROOF_OPERATION_DETAIL_870EBF",
        &mut errors,
    );

    let has_resisted_intel_map = snapshot.intel_map.iter().any(|row| row.resisted);
    let has_unresisted_intel_map = snapshot.intel_map.iter().any(|row| !row.resisted);
    if has_resisted_intel_map || !has(source_proofs, SOURCE_PROOF_INTEL_MAP_PERCENTAGES_870EBF) {
        snapshot.status_flags |= STATUS_INTEL_MAP_RESISTED;
    }
    require_rows(
        has_unresisted_intel_map,
        source_proofs,
        SOURCE_PROOF_INTEL_MAP_PERCENTAGES_870EBF,
        "unresisted intel-map rows require SOURCE_PROOF_INTEL_MAP_PERCENTAGES_870EBF",
        &mut errors,
    );

    let validation_us = duration_micros_u64_saturating(validation_started.elapsed());

    let encode_started = Instant::now();
    let payload = encode_payload(&snapshot);
    let encode_us = duration_micros_u64_saturating(encode_started.elapsed());

    // W2 production path: derive the gate summary ENCODE-SIDE from the snapshot
    // (offset 0 matches the pre-W2 `decode_payload(0, &payload)`), removing the
    // per-frame re-parse of the ~380k-row payload.
    let summary_started = Instant::now();
    let gate_summary = summarize_snapshot(0, &snapshot);
    let summary_us = duration_micros_u64_saturating(summary_started.elapsed());

    // W2 sampled cross-check: on 1/N frames, run the full decode and assert the
    // encode-side summary equals the decode-side one. On mismatch, FAIL CLOSED
    // (push an error → `report.ok()` false → collector nukes to SOURCE_UNAVAILABLE)
    // exactly as a decode failure did before W2, so an encoder regression / heap
    // tear that desyncs bytes from the snapshot is still caught. `decode_gate_us`
    // is 0 on off-sample frames.
    let mut decode_gate_us = 0u64;
    if sample_decode {
        let decode_gate_started = Instant::now();
        let decode_summary = decode_payload(0, &payload);
        decode_gate_us = duration_micros_u64_saturating(decode_gate_started.elapsed());
        if let Some(mismatch) = decode_verify_mismatch_error(&gate_summary, &decode_summary) {
            errors.push(mismatch);
        }
    }

    errors.extend(gate_summary.parse_errors.iter().cloned());

    Domain53ExtractionReport {
        domain_id: DOMAIN_ID_PROPOSED,
        source_proofs,
        snapshot,
        payload,
        gate_summary,
        errors,
        payload_phase_times: PayloadPhaseTimes {
            snapshot_us,
            validation_us,
            encode_us,
            summary_us,
            decode_gate_us,
        },
    }
}

pub fn proof_fixture_rows() -> ExtractedDomain53Rows {
    ExtractedDomain53Rows {
        elapsed_wall_ms: 3_600_000,
        changed_rows: None,
        countries: vec![CountryDiplomacyRow {
            tag_db_id: 1,
            faction_group: 42,
            at_war_count: 0,
            casualties: 0,
        }],
        wars: Vec::new(),
        opinions: vec![OpinionRow {
            src_tag_db_id: 1,
            target_tag_db_id: 3,
            opinion: -35,
        }],
        relation_flags: vec![RelationFlagRow {
            src_tag_db_id: 1,
            target_tag_db_id: 3,
            flag_kind: REL_KIND_GUARANTEE,
            directed: true,
        }],
        subjects: vec![SubjectRow {
            overlord_tag_db_id: 1,
            subject_tag_db_id: 22,
            autonomy_state_hash: crate::compact::stable_hash32("autonomy_puppet"),
            autonomy_ratio_raw: 40_000,
            start_date_raw: 60_795_475,
        }],
        agencies: vec![AgencyRow {
            owner_tag_db_id: 1,
            max_operatives: 3,
            usable_slots: 3,
            operative_count: 3,
            cryptology_target_count: 1,
            defense_raw: 600_000,
            upgrade_progress_raw: 2_400_000,
            name_hash: crate::compact::stable_hash64("Abwehr"),
            icon_hash: crate::compact::stable_hash64("GFX_intelligence_agency_logo_ger"),
        }],
        agency_capacity_clusters: vec![AgencyCapacityClusterRow {
            owner_tag_db_id: 2,
            value_0xe4: 9,
            value_0xf0: 10,
            value_0xf4: 9,
            proof_hash: crate::compact::stable_hash32("agency_capacity_offsets_e4_f0_f4"),
        }],
        operatives: vec![
            OperativeRow {
                owner_tag_db_id: 1,
                op_id: 11_387,
                detected: false,
                captured: false,
                skill: Some(2),
                experience_raw: Some(71_372_000),
                name_hash: crate::compact::stable_hash64("Johann Hein"),
                state_hash: crate::compact::stable_hash32("lifeflag_47"),
                lifecycle_flag: Some(47),
            },
            OperativeRow {
                owner_tag_db_id: 1,
                op_id: 11_388,
                detected: false,
                captured: false,
                skill: Some(1),
                experience_raw: Some(12_400_000),
                name_hash: crate::compact::stable_hash64("Pool Cohort"),
                state_hash: crate::compact::stable_hash32("lifeflag_31"),
                lifecycle_flag: Some(31),
            },
            OperativeRow {
                owner_tag_db_id: 1,
                op_id: 11_389,
                detected: false,
                captured: false,
                skill: Some(1),
                experience_raw: Some(0),
                name_hash: crate::compact::stable_hash64("Lifecycle Cohort"),
                state_hash: crate::compact::stable_hash32("lifeflag_63"),
                lifecycle_flag: Some(63),
            },
        ],
        cryptology: vec![CryptologyRow {
            owner_tag_db_id: 1,
            target_tag_db_id: 3,
            amount_raw: 960_000_000,
            decrypting: true,
        }],
        operation_anchors: Vec::new(),
        intel_map: Vec::new(),
        state_events: validated_state_event_rows(),
        operation_instances: vec![
            OperationInstanceRow {
                instance_id: 129_014,
                def_ref_ok: true,
                associated_country_tag_db_id: Some(3),
            },
            OperationInstanceRow {
                instance_id: 128_904,
                def_ref_ok: true,
                associated_country_tag_db_id: None,
            },
        ],
        incoming_actions: vec![
            IncomingActionQueueRow {
                recipient_tag_db_id: 9,
                actor_tag_db_id: 1,
                action_id: 150_442,
                action_type: 0,
                date_hours: 60_830_000,
                last_command_date_hours: 60_830_000,
            },
            IncomingActionQueueRow {
                recipient_tag_db_id: 10,
                actor_tag_db_id: 16,
                action_id: 150_608,
                action_type: 0,
                date_hours: 60_830_024,
                last_command_date_hours: 60_830_024,
            },
        ],
        intel_network_graphs: vec![
            IntelNetworkGraphRow {
                strategic_operative_index: 1,
                network_slot: 0,
                target_tag_db_id: 2,
                state_ids: vec![
                    121, 122, 123, 125, 126, 127, 128, 129, 130, 131, 132, 133, 338, 931,
                ],
            },
            IntelNetworkGraphRow {
                strategic_operative_index: 1,
                network_slot: 1,
                target_tag_db_id: 3,
                state_ids: vec![219, 253, 254],
            },
        ],
        factions: vec![FactionRow {
            faction_id: 1,
            faction_type: profile::DOMAIN53_FACTION_TYPE_EXPECTED,
            name_hash: crate::compact::stable_hash32("Axis"),
            leader_tag_db_id: 1,
            member_tag_db_ids: vec![1, 3, 22],
        }],
    }
}

fn validated_state_event_rows() -> Vec<StateEventRow> {
    vec![
        StateEventRow {
            owner_tag_db_id: 1,
            target_tag_db_id: Some(3),
            event_kind: STATE_EVENT_KIND_OBSERVED,
            source_kind: STATE_EVENT_SOURCE_RELATION_FLAG,
            object_hash: crate::compact::stable_hash32("relation_flag_guarantee"),
            value_hash: crate::compact::stable_hash32("directed_true"),
            value_raw: REL_KIND_GUARANTEE as i64,
        },
        StateEventRow {
            owner_tag_db_id: 1,
            target_tag_db_id: Some(22),
            event_kind: STATE_EVENT_KIND_OBSERVED,
            source_kind: STATE_EVENT_SOURCE_SUBJECT_AUTONOMY,
            object_hash: crate::compact::stable_hash32("autonomy_puppet"),
            value_hash: crate::compact::stable_hash32("autonomy_ratio_40000"),
            value_raw: 40_000,
        },
        StateEventRow {
            owner_tag_db_id: 1,
            target_tag_db_id: Some(3),
            event_kind: STATE_EVENT_KIND_OBSERVED,
            source_kind: STATE_EVENT_SOURCE_RELATION_OPINION,
            object_hash: crate::compact::stable_hash32("relation_opinion"),
            value_hash: crate::compact::stable_hash32("opinion_neg35"),
            value_raw: -35,
        },
        StateEventRow {
            owner_tag_db_id: 1,
            target_tag_db_id: None,
            event_kind: STATE_EVENT_KIND_OBSERVED,
            source_kind: STATE_EVENT_SOURCE_COUNTRY_FACTION_GROUP,
            object_hash: crate::compact::stable_hash32("country_faction_group"),
            value_hash: crate::compact::stable_hash32("faction_group_42"),
            value_raw: 42,
        },
        StateEventRow {
            owner_tag_db_id: 1,
            target_tag_db_id: None,
            event_kind: STATE_EVENT_KIND_OBSERVED,
            source_kind: STATE_EVENT_SOURCE_AGENCY_SCALAR,
            object_hash: crate::compact::stable_hash32("agency_upgrade_progress_raw"),
            value_hash: crate::compact::stable_hash32("agency_upgrade_progress_2400000"),
            value_raw: 2_400_000,
        },
        StateEventRow {
            owner_tag_db_id: 1,
            target_tag_db_id: None,
            event_kind: STATE_EVENT_KIND_OBSERVED,
            source_kind: STATE_EVENT_SOURCE_OPERATIVE_LIFECYCLE,
            object_hash: crate::compact::stable_hash32("operative_11387_lifecycle"),
            value_hash: crate::compact::stable_hash32("lifeflag_47"),
            value_raw: 47,
        },
        StateEventRow {
            owner_tag_db_id: 2,
            target_tag_db_id: None,
            event_kind: STATE_EVENT_KIND_OBSERVED,
            source_kind: STATE_EVENT_SOURCE_AGENCY_CAPACITY,
            object_hash: crate::compact::stable_hash32("agency_capacity_offsets_e4_f0_f4"),
            value_hash: crate::compact::stable_hash32("agency_capacity_9_10_9"),
            value_raw: 10,
        },
        StateEventRow {
            owner_tag_db_id: 1,
            target_tag_db_id: Some(3),
            event_kind: STATE_EVENT_KIND_OBSERVED,
            source_kind: STATE_EVENT_SOURCE_CRYPTOLOGY,
            object_hash: crate::compact::stable_hash32("cryptology_amount_raw"),
            value_hash: crate::compact::stable_hash32("cryptology_amount_960000000"),
            value_raw: 960_000_000,
        },
    ]
}

fn require_rows(
    present: bool,
    source_proofs: u64,
    required: u64,
    message: &str,
    errors: &mut Vec<String>,
) {
    if present && !has(source_proofs, required) {
        errors.push(message.to_string());
    }
}

fn has(source_proofs: u64, required: u64) -> bool {
    source_proofs & required != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    // T3 sweep (perf_rearch_20260706): the runtime prefetch-distance override
    // parses through `parse_prefetch_dist`. Test the PURE parse (not the live
    // env read) so the assertion is deterministic and does not mutate the
    // process environment (parallel test env mutation is unsafe). Contract:
    // fall back to `default` on None / empty / garbage; return the parsed value
    // otherwise; PRESERVE 0 (the "prefetch OFF" arm). Perf-only: the result
    // only changes which prefetch hints issue, never rows/bytes/errors.
    #[test]
    fn parse_prefetch_dist_falls_back_when_absent_or_garbage() {
        // Unset -> default.
        assert_eq!(parse_prefetch_dist(None, 16), 16);
        assert_eq!(parse_prefetch_dist(None, 32), 32);
        // Empty / whitespace-only -> default (not parseable as usize).
        assert_eq!(parse_prefetch_dist(Some(""), 16), 16);
        assert_eq!(parse_prefetch_dist(Some("   "), 32), 32);
        // Garbage -> default.
        assert_eq!(parse_prefetch_dist(Some("abc"), 16), 16);
        assert_eq!(parse_prefetch_dist(Some("16x"), 16), 16);
        assert_eq!(parse_prefetch_dist(Some("-4"), 32), 32);
        assert_eq!(parse_prefetch_dist(Some("3.5"), 32), 32);
    }

    #[test]
    fn parse_prefetch_dist_returns_parsed_value() {
        assert_eq!(parse_prefetch_dist(Some("8"), 16), 8);
        assert_eq!(parse_prefetch_dist(Some("64"), 32), 64);
        // Surrounding whitespace is trimmed before parsing.
        assert_eq!(parse_prefetch_dist(Some("  24  "), 16), 24);
        // A parsed value overrides the default even when it equals a common one.
        assert_eq!(parse_prefetch_dist(Some("16"), 32), 16);
    }

    #[test]
    fn parse_prefetch_dist_honors_zero_disable() {
        // 0 is a VALID parsed value (prefetch OFF), NOT treated as garbage — it
        // must NOT fall back to the default.
        assert_eq!(parse_prefetch_dist(Some("0"), 16), 0);
        assert_eq!(parse_prefetch_dist(Some("0"), 32), 0);
        assert_eq!(parse_prefetch_dist(Some(" 0 "), 32), 0);
    }

    // P3: the shared-sweep needle order MUST line up with the field order
    // `domain53_discovery_from_needle_hits` assigns, or discovery is silently
    // corrupted. Lock both: `domain53_scan_needles[i]` is the absolute vtable
    // for the i-th argument of `domain53_discovery_from_needle_hits`.
    #[test]
    fn domain53_scan_needles_order_matches_discovery_fields() {
        let module_base = 0x1_4000_0000u64;
        let needles = domain53_scan_needles(module_base).expect("needles");
        assert_eq!(
            needles[0],
            add_addr(module_base, profile::DOMAIN53_OPERATIVE_VTABLE_RVA).unwrap()
        );
        let date_vtables = domain53_war_date_vtables(module_base).unwrap();
        assert_eq!(needles[1], date_vtables[0]);
        assert_eq!(needles[2], date_vtables[1]);
        assert_eq!(
            needles[3],
            add_addr(module_base, profile::DOMAIN53_OPERATION_INSTANCE_VTABLE_RVA).unwrap()
        );
        assert_eq!(
            needles[4],
            add_addr(
                module_base,
                profile::DOMAIN53_STRATEGIC_OPERATIVE_VTABLE_RVA
            )
            .unwrap()
        );
        assert_eq!(
            needles[5],
            add_addr(
                module_base,
                profile::DOMAIN53_COUNTRY_INTEL_NETWORK_VTABLE_RVA
            )
            .unwrap()
        );

        // The from-hits builder assigns each argument to the matching field, in
        // the same positional order as the needle array.
        let discovery = domain53_discovery_from_needle_hits(
            vec![1],
            vec![2],
            vec![3],
            vec![4],
            vec![5],
            vec![6],
        );
        assert_eq!(discovery.operative_hits, vec![1]);
        assert_eq!(discovery.war_hits[0], vec![2]);
        assert_eq!(discovery.war_hits[1], vec![3]);
        assert_eq!(discovery.operation_instance_hits, vec![4]);
        assert_eq!(discovery.strategic_operative_hits, vec![5]);
        assert_eq!(discovery.country_intel_network_hits, vec![6]);
    }

    #[test]
    fn domain53_scan_needles_rejects_zero_module_base() {
        assert!(domain53_scan_needles(0).is_err());
    }

    const FAKE_COUNTRY_ROWS: u32 = 45;
    const TEST_COUNTRY_COL_RVA: u64 = 0x3600_0000;
    const TEST_COUNTRY_TYPE_DESCRIPTOR_RVA: u64 = 0x3600_1000;
    const TEST_AGENCY_COL_RVA: u64 = 0x3600_2000;
    const TEST_AGENCY_TYPE_DESCRIPTOR_RVA: u64 = 0x3600_3000;
    const TEST_RTTI_CCOUNTRY_DECORATED: &str = ".?AVCCountry@@";
    const TEST_RTTI_CINTELLIGENCE_AGENCY_DECORATED: &str = ".?AVCIntelligenceAgency@@";

    #[derive(Clone, Debug)]
    struct FakeMemory {
        module_base: u64,
        chunks: Vec<(u64, Vec<u8>)>,
        ranges: Vec<(u64, usize)>,
    }

    impl FakeMemory {
        fn new(module_base: u64) -> Self {
            Self {
                module_base,
                chunks: Vec::new(),
                ranges: Vec::new(),
            }
        }

        fn add_chunk(&mut self, address: u64, len: usize) {
            self.chunks.push((address, vec![0; len]));
        }

        fn add_range(&mut self, address: u64, len: usize) {
            self.ranges.push((address, len));
        }

        fn chunk_mut(&mut self, address: u64, len: usize) -> &mut [u8] {
            for (base, bytes) in &mut self.chunks {
                let end = *base + bytes.len() as u64;
                if address >= *base && address + len as u64 <= end {
                    let offset = (address - *base) as usize;
                    return &mut bytes[offset..offset + len];
                }
            }
            panic!("missing fake memory chunk at {address:#x} len {len}");
        }

        fn write_u8(&mut self, address: u64, value: u8) {
            self.chunk_mut(address, 1)[0] = value;
        }

        fn write_u32(&mut self, address: u64, value: u32) {
            self.chunk_mut(address, 4)
                .copy_from_slice(&value.to_le_bytes());
        }

        fn write_i32(&mut self, address: u64, value: i32) {
            self.chunk_mut(address, 4)
                .copy_from_slice(&value.to_le_bytes());
        }

        fn write_u64(&mut self, address: u64, value: u64) {
            self.chunk_mut(address, 8)
                .copy_from_slice(&value.to_le_bytes());
        }

        fn write_i64(&mut self, address: u64, value: i64) {
            self.chunk_mut(address, 8)
                .copy_from_slice(&value.to_le_bytes());
        }

        fn write_bytes(&mut self, address: u64, value: &[u8]) {
            self.chunk_mut(address, value.len()).copy_from_slice(value);
        }

        fn write_inline_string(&mut self, address: u64, value: &str) {
            let bytes = value.as_bytes();
            assert!(bytes.len() < 16);
            self.chunk_mut(address, 16).fill(0);
            self.chunk_mut(address, bytes.len()).copy_from_slice(bytes);
            self.write_u64(address + 0x10, bytes.len() as u64);
            self.write_u64(address + 0x18, 15);
        }

        fn write_rtti(&mut self, vtable_rva: usize, col_rva: u64, td_rva: u64, name: &str) {
            let module = self.module_base;
            let vtable = module + vtable_rva as u64;
            let col = module + col_rva;
            let td = module + td_rva;
            self.add_chunk(vtable - 8, 8);
            self.add_chunk(col, 24);
            self.add_chunk(td, 0x10 + DOMAIN53_RTTI_MAX_NAME_LEN);
            self.write_u64(vtable - 8, col);
            self.write_u32(col, 1);
            self.write_u32(col + 0x0C, td_rva as u32);
            self.write_u32(col + 0x10, 0x3700_0000);
            self.write_u32(col + 0x14, col_rva as u32);
            self.write_rtti_name(td_rva, name);
        }

        fn write_rtti_name(&mut self, td_rva: u64, name: &str) {
            let td = self.module_base + td_rva;
            let len = name.len() + 1;
            self.chunk_mut(td + 0x10, len).fill(0);
            self.write_bytes(td + 0x10, name.as_bytes());
        }
    }

    impl Domain53Memory for FakeMemory {
        fn module_base(&self) -> u64 {
            self.module_base
        }

        fn read_bytes(&self, address: u64, len: usize) -> Result<Vec<u8>, ExtractError> {
            for (base, bytes) in &self.chunks {
                let end = *base + bytes.len() as u64;
                if address >= *base && address + len as u64 <= end {
                    let offset = (address - *base) as usize;
                    return Ok(bytes[offset..offset + len].to_vec());
                }
            }
            Err(ExtractError::MemoryRead { address, len })
        }

        fn readable_ranges(&self, _lo: u64, _hi: u64) -> Vec<(u64, usize)> {
            self.ranges.clone()
        }
    }

    fn fake_country_addr(heap: u64, tag: u32) -> u64 {
        heap + 0x20_0000 + u64::from(tag) * 0x2000
    }

    fn fake_domain53_memory(include_relations: bool, include_operative: bool) -> FakeMemory {
        let module = 0x1400_0000_0;
        let heap = profile::DOMAIN53_HEAP_LO as u64 + 0x10_0000;
        let game_state = heap;
        let country_database = heap + 0x1_0000;
        let country_table = heap + 0x2_0000;
        let country1 = heap + 0x3_0000;
        let country2 = heap + 0x4_0000;
        let country3 = heap + 0x5_0000;
        let rel_manager1 = heap + 0x6_0000;
        let rel_table1 = heap + 0x7_0000;
        let rel12 = heap + 0x8_0000;
        let rel13 = heap + 0x9_0000;
        let autonomy_state = heap + 0xA_0000;
        let agency1 = heap + 0xB_0000;
        let crypto1 = heap + 0xC_0000;
        let crypto_records = heap + 0xD_0000;
        let leader1 = heap + 0xE_0000;
        let skill1 = heap + 0xF_0000;
        let war1 = heap + 0x10_0000;
        let incoming_queue2 = heap + 0x11_0000;
        let incoming_data2 = heap + 0x12_0000;
        let incoming_action2 = heap + 0x13_0000;
        let incoming_body2 = heap + 0x14_0000;
        let strategic_operative1 = heap + 0x15_0000;
        let strategic_network_vec1 = heap + 0x16_0000;
        let intel_network1 = heap + 0x17_0000;
        let intel_graph1 = heap + 0x18_0000;
        let intel_vertices1 = heap + 0x19_0000;
        let subject_object = heap + 0x1A_0000;
        let faction_system = heap + 0x1B_0000;
        let faction_array = heap + 0x1C_0000;
        let faction1 = heap + 0x1D_0000;
        let faction_members = heap + 0x1E_0000;

        let mut mem = FakeMemory::new(module);
        mem.write_rtti(
            profile::DOMAIN53_COUNTRY_VTABLE_RVA,
            TEST_COUNTRY_COL_RVA,
            TEST_COUNTRY_TYPE_DESCRIPTOR_RVA,
            TEST_RTTI_CCOUNTRY_DECORATED,
        );
        mem.write_rtti(
            profile::DOMAIN53_AGENCY_VTABLE_RVA,
            TEST_AGENCY_COL_RVA,
            TEST_AGENCY_TYPE_DESCRIPTOR_RVA,
            TEST_RTTI_CINTELLIGENCE_AGENCY_DECORATED,
        );
        for (addr, len) in [
            (module + profile::CG_STATE_DATE_OFFSET as u64, 8),
            (module + profile::COUNTRY_DATABASE_OFFSET as u64, 8),
            (game_state, 0x500),
            (country_database, 0x100),
            (country_table, (FAKE_COUNTRY_ROWS as usize + 1) * 8),
            (country1, 0x1100),
            (country2, 0x1100),
            (country3, 0x1100),
            (rel_manager1, 0x40),
            (rel_table1, (FAKE_COUNTRY_ROWS as usize + 1) * 8),
            (rel12, profile::DOMAIN53_RELATION_SCAN_BYTES),
            (rel13, profile::DOMAIN53_RELATION_SCAN_BYTES),
            (autonomy_state, 0x80),
            (agency1, 0x180),
            (crypto1, 0x80),
            (crypto_records, profile::DOMAIN53_CRYPTOLOGY_RECORD_STRIDE),
            (skill1, 0x200),
            (war1, 0x80),
            (incoming_queue2, 0x40),
            (incoming_data2, 0x08),
            (incoming_action2, 0x20),
            (incoming_body2, 0x50),
            (strategic_operative1, 0x40),
            (strategic_network_vec1, 0x08),
            (intel_network1, 0x30),
            (intel_graph1, 0x60),
            (
                intel_vertices1,
                profile::DOMAIN53_INTEL_NETWORK_GRAPH_VERTEX_STRIDE * 3,
            ),
            (subject_object, 0x80),
            (faction_system, 0x40),
            (faction_array, 0x20),
            (faction1, 0x80),
            (faction_members, 0x20),
        ] {
            mem.add_chunk(addr, len);
        }
        if include_operative {
            mem.add_chunk(leader1, 0xF00);
            mem.add_range(leader1, 0xF00);
        }
        for tag in 4..=FAKE_COUNTRY_ROWS {
            mem.add_chunk(fake_country_addr(heap, tag), 0x1100);
        }
        mem.add_range(war1, 0x80);
        mem.add_range(strategic_operative1, 0x40);
        mem.add_range(intel_network1, 0x30);

        mem.write_u64(module + profile::CG_STATE_DATE_OFFSET as u64, game_state);
        mem.write_u32(
            game_state + profile::CGAMESTATE_CURRENT_HOUR as u64,
            60_830_441,
        );
        mem.write_u64(
            module + profile::COUNTRY_DATABASE_OFFSET as u64,
            country_database,
        );
        mem.write_u64(
            game_state + profile::DOMAIN53_CGAMESTATE_COUNTRY_TABLE as u64,
            country_table,
        );
        mem.write_u32(
            game_state + profile::DOMAIN53_CGAMESTATE_COUNTRY_COUNT as u64,
            FAKE_COUNTRY_ROWS + 1,
        );
        for tag in 1..=FAKE_COUNTRY_ROWS {
            let country = match tag {
                1 => country1,
                2 => country2,
                3 => country3,
                _ => fake_country_addr(heap, tag),
            };
            mem.write_u64(country_table + u64::from(tag) * 8, country);
            mem.write_u64(
                country,
                module + profile::DOMAIN53_COUNTRY_VTABLE_RVA as u64,
            );
            mem.write_u32(country + profile::DOMAIN53_OBJ_TAG_DB_ID as u64, tag);
            let faction_group = match tag {
                1..=20 => 1,
                21..=35 => 2,
                _ => 3,
            };
            mem.write_i32(
                country + profile::DOMAIN53_COUNTRY_FACTION_GROUP as u64,
                faction_group,
            );
        }

        // CFactionSystem: one faction (id=1, type=88) with members [country1,
        // country2]; leader = members[0] = country1 (tag 1). Array count packs
        // {size=1, capacity=1} at +0x28.
        mem.write_u64(
            game_state + profile::DOMAIN53_CGAMESTATE_FACTION_SYSTEM as u64,
            faction_system,
        );
        mem.write_u64(
            faction_system + profile::DOMAIN53_FACTION_ARRAY as u64,
            faction_array,
        );
        mem.write_u64(
            faction_system + profile::DOMAIN53_FACTION_COUNT as u64,
            (1u64 << 32) | 1u64,
        );
        mem.write_u64(faction_array, faction1);
        // id = HI32, type = LO32.
        mem.write_u64(
            faction1 + profile::DOMAIN53_FACTION_ID as u64,
            (1u64 << 32) | u64::from(profile::DOMAIN53_FACTION_TYPE_EXPECTED),
        );
        mem.write_inline_string(faction1 + profile::DOMAIN53_FACTION_NAME as u64, "Axis");
        mem.write_u64(
            faction1 + profile::DOMAIN53_FACTION_MEMBERS as u64,
            faction_members,
        );
        mem.write_u32(faction1 + profile::DOMAIN53_FACTION_MEMBERS_SIZE as u64, 2);
        mem.write_u64(faction_members, country1);
        mem.write_u64(faction_members + 8, country2);

        if include_relations {
            mem.write_u64(
                country1 + profile::DOMAIN53_RELATION_MANAGER as u64,
                rel_manager1,
            );
            mem.write_u64(
                rel_manager1 + profile::DOMAIN53_RELATION_TABLE as u64,
                rel_table1,
            );
            mem.write_u64(rel_table1 + 2 * 8, rel12);
            mem.write_u64(rel_table1 + 3 * 8, rel13);
            // rel12 is the CRelationStatus for country1->country2. The subject
            // (puppet) relation is NOT inline: rel12+0x288 points to a separate
            // heap CSubjectRelation object.
            mem.write_i32(rel12 + profile::DOMAIN53_RELATION_OPINION as u64, 10);
            mem.write_u64(
                rel12 + profile::DOMAIN53_RELATION_SUBJECT_PTR as u64,
                subject_object,
            );
            mem.write_u64(
                subject_object,
                module + profile::DOMAIN53_REL_SUBJECT_VTABLE_RVA as u64,
            );
            mem.write_u32(
                subject_object + profile::DOMAIN53_SUBJECT_OVERLORD as u64,
                1,
            );
            mem.write_u32(subject_object + profile::DOMAIN53_SUBJECT_SUBJECT as u64, 2);
            mem.write_u64(
                subject_object + profile::DOMAIN53_SUBJECT_STATE_DB as u64,
                autonomy_state,
            );
            mem.write_i32(
                subject_object + profile::DOMAIN53_SUBJECT_RATIO as u64,
                40_000,
            );
            // start_date raw = 1940.2.14.20 (matches the GER->DEN melt subject).
            mem.write_i32(
                subject_object + profile::DOMAIN53_SUBJECT_START_DATE as u64,
                60_795_475,
            );
            mem.write_inline_string(
                autonomy_state + profile::DOMAIN53_SUBJECT_STATE_NAME as u64,
                "aut_puppet",
            );

            // rel13 (country1 -> country3): a GUARANTEE relation carried in the pair's
            // sub-relation container (`pdx::vector<CRelation*>` at rel13+0xD0), NOT inline
            // (the inline vtable scan is gone). Exercises `read_relation_container`
            // end-to-end: begin=guarantee_arr, count=1, one CGuaranteeRelation object.
            let guarantee_arr = heap + 0x1F_0000;
            let guarantee_obj = heap + 0x1F_8000;
            mem.add_chunk(guarantee_arr, 0x40);
            mem.add_chunk(guarantee_obj, 0x40);
            mem.write_u64(
                rel13 + profile::DOMAIN53_RELATION_SUBREL_BEGIN as u64,
                guarantee_arr,
            );
            mem.write_i32(rel13 + profile::DOMAIN53_RELATION_SUBREL_CAP as u64, 1);
            mem.write_i32(rel13 + profile::DOMAIN53_RELATION_SUBREL_COUNT as u64, 1);
            mem.write_u64(guarantee_arr, guarantee_obj);
            mem.write_u64(
                guarantee_obj,
                module + profile::DOMAIN53_REL_GUARANTEE_VTABLE_RVA as u64,
            );
            mem.write_u32(guarantee_obj + profile::DOMAIN53_REL_BASE_FIRST as u64, 1);
            mem.write_u32(guarantee_obj + profile::DOMAIN53_REL_BASE_SECOND as u64, 3);
            // start_date raw = a valid hour before the fixture clock (60_830_441);
            // end_date = sentinel (guarantees carry no end_date in the save).
            mem.write_i32(
                guarantee_obj + profile::DOMAIN53_REL_BASE_START_DATE as u64,
                60_000_000,
            );
            mem.write_i32(
                guarantee_obj + profile::DOMAIN53_REL_BASE_END_DATE as u64,
                profile::DOMAIN53_REL_DATE_SENTINEL,
            );
            mem.write_i32(rel13 + profile::DOMAIN53_RELATION_OPINION as u64, -35);
        }

        mem.write_u64(
            country1 + profile::DOMAIN53_COUNTRY_AGENCY_PTR as u64,
            agency1,
        );
        mem.write_u64(agency1, module + profile::DOMAIN53_AGENCY_VTABLE_RVA as u64);
        mem.write_inline_string(agency1 + profile::DOMAIN53_AGENCY_NAME as u64, "Abwehr");
        mem.write_inline_string(agency1 + profile::DOMAIN53_AGENCY_ICON as u64, "GFX_ger");
        mem.write_i32(
            agency1 + profile::DOMAIN53_AGENCY_UPGRADE_PROGRESS as u64,
            2_400_000,
        );
        mem.write_u32(agency1 + profile::DOMAIN53_AGENCY_CAPACITY_E4 as u64, 9);
        mem.write_u32(agency1 + profile::DOMAIN53_AGENCY_CAPACITY_F0 as u64, 10);
        mem.write_u32(agency1 + profile::DOMAIN53_AGENCY_CAPACITY_F4 as u64, 9);
        mem.write_u64(
            agency1 + profile::DOMAIN53_AGENCY_CRYPTO_MGR as u64,
            crypto1,
        );
        mem.write_i32(agency1 + profile::DOMAIN53_AGENCY_DEFENSE as u64, 600_000);

        mem.write_u64(
            crypto1,
            module + profile::DOMAIN53_CRYPTOLOGY_VTABLE_RVA as u64,
        );
        mem.write_u64(
            crypto1 + profile::DOMAIN53_CRYPTOLOGY_MGR_DATA as u64,
            crypto_records,
        );
        mem.write_u32(crypto1 + profile::DOMAIN53_CRYPTOLOGY_MGR_SIZE as u64, 1);
        mem.write_u32(crypto1 + profile::DOMAIN53_CRYPTOLOGY_MGR_CAP as u64, 1);
        mem.write_u64(
            crypto_records,
            module + profile::DOMAIN53_CRYPTOLOGY_RECORD_VTABLE_RVA as u64,
        );
        mem.write_u32(
            crypto_records + profile::DOMAIN53_CRYPTOLOGY_RECORD_TARGET as u64,
            3,
        );
        mem.write_u8(
            crypto_records + profile::DOMAIN53_CRYPTOLOGY_RECORD_DECRYPTING as u64,
            1,
        );
        mem.write_i64(
            crypto_records + profile::DOMAIN53_CRYPTOLOGY_RECORD_AMOUNT as u64,
            960_000_000,
        );

        if include_operative {
            let identity = leader1 + profile::DOMAIN53_OPERATIVE_IDENTITY_FROM_VTABLE as u64;
            mem.write_u64(
                leader1,
                module + profile::DOMAIN53_OPERATIVE_VTABLE_RVA as u64,
            );
            mem.write_u32(identity, profile::DOMAIN53_OPERATIVE_HANDLE_TYPE);
            mem.write_u32(identity + 4, 11_387);
            mem.write_inline_string(identity + profile::DOMAIN53_OPERATIVE_NAME as u64, "Johann");
            mem.write_inline_string(
                identity + profile::DOMAIN53_OPERATIVE_NAME_DUPLICATE as u64,
                "Johann",
            );
            mem.write_u8(identity + profile::DOMAIN53_OPERATIVE_DETECTED as u64, 0);
            mem.write_u32(
                identity + profile::DOMAIN53_OPERATIVE_LIFECYCLE_FLAG as u64,
                47,
            );
            mem.write_u32(identity + profile::DOMAIN53_OPERATIVE_OWNER as u64, 1);
            mem.write_u64(
                leader1 + profile::DOMAIN53_OPERATIVE_SKILL_SUBOBJECT as u64,
                skill1,
            );
            mem.write_i64(
                leader1 + profile::DOMAIN53_OPERATIVE_EXPERIENCE_RAW as u64,
                71_372_000,
            );
            mem.write_u32(skill1 + profile::DOMAIN53_OPERATIVE_SKILL_LEVEL as u64, 2);
        }

        mem.write_u32(war1 + profile::DOMAIN53_WAR_FIRST_TAG as u64, 1);
        mem.write_u32(war1 + profile::DOMAIN53_WAR_SECOND_TAG as u64, 3);
        mem.write_u64(
            war1 + profile::DOMAIN53_WAR_DATE_VTABLE as u64,
            module + profile::DOMAIN53_WAR_GAME_DATE_VTABLE_RVA as u64,
        );
        mem.write_i32(war1 + profile::DOMAIN53_WAR_FIRST_CASUALTIES as u64, 100);
        mem.write_i32(war1 + profile::DOMAIN53_WAR_SECOND_CASUALTIES as u64, 200);
        mem.write_u32(war1 + profile::DOMAIN53_WAR_ECHO_FIRST as u64, 1);
        mem.write_u32(war1 + profile::DOMAIN53_WAR_ECHO_SECOND as u64, 3);

        mem.write_u64(
            country2 + profile::DOMAIN53_INCOMING_ACTION_QUEUE as u64,
            incoming_queue2,
        );
        mem.write_u64(
            incoming_queue2 + profile::DOMAIN53_INCOMING_ACTION_QUEUE_DATA as u64,
            incoming_data2,
        );
        mem.write_u64(
            incoming_queue2 + profile::DOMAIN53_INCOMING_ACTION_QUEUE_PACKED_SIZE_CAP as u64,
            (1u64 << 32) | 1,
        );
        mem.write_u32(
            incoming_queue2 + profile::DOMAIN53_INCOMING_ACTION_QUEUE_OWNER_GUARD as u64,
            2,
        );
        mem.write_u64(incoming_data2, incoming_action2);
        mem.write_u64(
            incoming_action2,
            module + profile::DOMAIN53_INCOMING_ACTION_VTABLE_RVA as u64,
        );
        mem.write_u64(
            incoming_action2 + profile::DOMAIN53_INCOMING_ACTION_HANDLE as u64,
            (150_442u64 << 32) | u64::from(profile::DOMAIN53_INCOMING_ACTION_HANDLE_TYPE),
        );
        mem.write_u64(
            incoming_action2 + profile::DOMAIN53_INCOMING_ACTION_BODY as u64,
            incoming_body2,
        );
        mem.write_i32(
            incoming_body2 + profile::DOMAIN53_INCOMING_ACTION_BODY_TYPE as u64,
            0,
        );
        mem.write_u32(
            incoming_body2 + profile::DOMAIN53_INCOMING_ACTION_ACTOR as u64,
            1,
        );
        mem.write_u32(
            incoming_body2 + profile::DOMAIN53_INCOMING_ACTION_ORIGINAL_ACTOR as u64,
            1,
        );
        mem.write_u32(
            incoming_body2 + profile::DOMAIN53_INCOMING_ACTION_RECIPIENT as u64,
            2,
        );
        mem.write_u32(
            incoming_body2 + profile::DOMAIN53_INCOMING_ACTION_ORIGINAL_RECIPIENT as u64,
            2,
        );
        mem.write_u32(
            incoming_body2 + profile::DOMAIN53_INCOMING_ACTION_DATE as u64,
            60_830_000,
        );
        mem.write_u32(
            incoming_body2 + profile::DOMAIN53_INCOMING_ACTION_LAST_COMMAND_DATE as u64,
            60_830_000,
        );

        mem.write_u64(
            strategic_operative1,
            module + profile::DOMAIN53_STRATEGIC_OPERATIVE_VTABLE_RVA as u64,
        );
        mem.write_u64(
            strategic_operative1 + profile::DOMAIN53_STRATEGIC_OPERATIVE_INDEX as u64,
            1,
        );
        mem.write_u64(
            strategic_operative1 + profile::DOMAIN53_STRATEGIC_OPERATIVE_NETWORK_VECTOR as u64,
            strategic_network_vec1,
        );
        mem.write_u32(
            strategic_operative1 + profile::DOMAIN53_STRATEGIC_OPERATIVE_NETWORK_SIZE as u64,
            1,
        );
        mem.write_u32(
            strategic_operative1 + profile::DOMAIN53_STRATEGIC_OPERATIVE_NETWORK_CAPACITY as u64,
            1,
        );
        mem.write_u64(strategic_network_vec1, intel_network1);
        mem.write_u64(
            intel_network1,
            module + profile::DOMAIN53_COUNTRY_INTEL_NETWORK_VTABLE_RVA as u64,
        );
        mem.write_i32(
            intel_network1 + profile::DOMAIN53_COUNTRY_INTEL_NETWORK_TARGET as u64,
            3,
        );
        mem.write_u64(
            intel_network1 + profile::DOMAIN53_COUNTRY_INTEL_NETWORK_GRAPH as u64,
            intel_graph1,
        );
        mem.write_u64(
            intel_graph1 + profile::DOMAIN53_INTEL_NETWORK_GRAPH_VERTEX_DATA as u64,
            intel_vertices1,
        );
        mem.write_u32(
            intel_graph1 + profile::DOMAIN53_INTEL_NETWORK_GRAPH_VERTEX_COUNT as u64,
            3,
        );
        for (index, state_id) in [219u32, 253, 254].iter().enumerate() {
            mem.write_i32(
                intel_vertices1
                    + (index * profile::DOMAIN53_INTEL_NETWORK_GRAPH_VERTEX_STRIDE) as u64
                    + profile::DOMAIN53_INTEL_NETWORK_GRAPH_VERTEX_STATE_ID as u64,
                *state_id as i32,
            );
        }

        mem
    }

    fn fake_heap_base() -> u64 {
        profile::DOMAIN53_HEAP_LO as u64 + 0x10_0000
    }

    fn add_fake_operative(mem: &mut FakeMemory, index: u32) -> u64 {
        let heap = fake_heap_base();
        let leader = heap + 0x30_0000 + u64::from(index) * 0x2000;
        let skill = leader + 0x1000;
        mem.add_chunk(leader, 0xF00);
        mem.add_chunk(skill, 0x200);
        mem.add_range(leader, 0xF00);

        let identity = leader + profile::DOMAIN53_OPERATIVE_IDENTITY_FROM_VTABLE as u64;
        mem.write_u64(
            leader,
            mem.module_base + profile::DOMAIN53_OPERATIVE_VTABLE_RVA as u64,
        );
        mem.write_u32(identity, profile::DOMAIN53_OPERATIVE_HANDLE_TYPE);
        mem.write_u32(identity + 4, 20_000 + index);
        let name = format!("Op{index}");
        mem.write_inline_string(identity + profile::DOMAIN53_OPERATIVE_NAME as u64, &name);
        mem.write_inline_string(
            identity + profile::DOMAIN53_OPERATIVE_NAME_DUPLICATE as u64,
            &name,
        );
        mem.write_u8(identity + profile::DOMAIN53_OPERATIVE_DETECTED as u64, 0);
        mem.write_u32(
            identity + profile::DOMAIN53_OPERATIVE_LIFECYCLE_FLAG as u64,
            47,
        );
        mem.write_u32(identity + profile::DOMAIN53_OPERATIVE_OWNER as u64, 1);
        mem.write_u64(
            leader + profile::DOMAIN53_OPERATIVE_SKILL_SUBOBJECT as u64,
            skill,
        );
        mem.write_i64(
            leader + profile::DOMAIN53_OPERATIVE_EXPERIENCE_RAW as u64,
            70_000_000 + i64::from(index),
        );
        mem.write_u32(skill + profile::DOMAIN53_OPERATIVE_SKILL_LEVEL as u64, 2);
        leader
    }

    fn add_fake_war(mem: &mut FakeMemory, index: u32) -> u64 {
        let heap = fake_heap_base();
        let war = heap + 0x40_0000 + u64::from(index) * 0x100;
        let second_tag = 10 + index;
        assert!(second_tag <= FAKE_COUNTRY_ROWS);
        mem.add_chunk(war, 0x80);
        mem.add_range(war, 0x80);

        mem.write_u32(war + profile::DOMAIN53_WAR_FIRST_TAG as u64, 1);
        mem.write_u32(war + profile::DOMAIN53_WAR_SECOND_TAG as u64, second_tag);
        mem.write_u64(
            war + profile::DOMAIN53_WAR_DATE_VTABLE as u64,
            mem.module_base + profile::DOMAIN53_WAR_GAME_DATE_VTABLE_RVA as u64,
        );
        mem.write_i32(
            war + profile::DOMAIN53_WAR_FIRST_CASUALTIES as u64,
            100 + index as i32,
        );
        mem.write_i32(
            war + profile::DOMAIN53_WAR_SECOND_CASUALTIES as u64,
            200 + index as i32,
        );
        mem.write_u32(war + profile::DOMAIN53_WAR_ECHO_FIRST as u64, 1);
        mem.write_u32(war + profile::DOMAIN53_WAR_ECHO_SECOND as u64, second_tag);
        war
    }

    fn total_war_hits(discovery: &Domain53Discovery) -> usize {
        discovery.war_hits.iter().map(Vec::len).sum()
    }

    #[test]
    fn reader_extracts_validated_slice_from_fake_memory() {
        let mem = fake_domain53_memory(true, true);
        let extracted = extract_rows_from_current_roots(&mem).expect("domain53 fake read");

        assert_eq!(extracted.skipped_rows, 0);
        assert_eq!(extracted.rows.countries.len(), FAKE_COUNTRY_ROWS as usize);
        assert_eq!(extracted.rows.opinions.len(), 2);
        // Guarantee flag (1->3, from the rel13 +0xD0 sub-relation container) +
        // synthesized puppet flag (1->2 from the +0x288 subject hop) = 2 relation flags.
        assert_eq!(extracted.rows.relation_flags.len(), 2);
        assert!(extracted.rows.relation_flags.iter().any(|row| {
            row.src_tag_db_id == 1 && row.target_tag_db_id == 2 && row.flag_kind == REL_KIND_PUPPET
        }));
        // The container-derived guarantee: correct kind + direction (first=1, second=3).
        assert!(extracted.rows.relation_flags.iter().any(|row| {
            row.src_tag_db_id == 1
                && row.target_tag_db_id == 3
                && row.flag_kind == REL_KIND_GUARANTEE
                && row.directed
        }));
        // Its start_date raw rides a relation-flag state_event.
        assert!(extracted.rows.state_events.iter().any(|e| e.source_kind
            == STATE_EVENT_SOURCE_RELATION_FLAG
            && e.value_raw == 60_000_000));
        assert_eq!(extracted.rows.subjects.len(), 1);
        // The +0x288 pointer hop carries the validated fields incl. start_date.
        assert_eq!(extracted.rows.subjects[0].overlord_tag_db_id, 1);
        assert_eq!(extracted.rows.subjects[0].subject_tag_db_id, 2);
        assert_eq!(extracted.rows.subjects[0].start_date_raw, 60_795_475);
        // CFactionSystem roster: one faction, leader = members[0].
        assert_eq!(extracted.rows.factions.len(), 1);
        assert_eq!(extracted.rows.factions[0].faction_id, 1);
        assert_eq!(extracted.rows.factions[0].faction_type, 88);
        assert_eq!(extracted.rows.factions[0].leader_tag_db_id, 1);
        assert_eq!(extracted.rows.factions[0].member_tag_db_ids, vec![1, 2]);
        assert_eq!(extracted.rows.agencies.len(), 1);
        assert_eq!(extracted.rows.agency_capacity_clusters.len(), 1);
        assert_eq!(extracted.rows.cryptology.len(), 1);
        assert_eq!(extracted.rows.operatives.len(), 1);
        assert_eq!(extracted.rows.wars.len(), 1);
        assert_eq!(
            extracted.rows.incoming_actions,
            vec![IncomingActionQueueRow {
                recipient_tag_db_id: 2,
                actor_tag_db_id: 1,
                action_id: 150_442,
                action_type: 0,
                date_hours: 60_830_000,
                last_command_date_hours: 60_830_000,
            }]
        );
        assert!(extracted.rows.operation_anchors.is_empty());
        assert!(extracted.rows.intel_map.is_empty());
        assert_eq!(
            extracted.rows.intel_network_graphs,
            vec![IntelNetworkGraphRow {
                strategic_operative_index: 1,
                network_slot: 0,
                target_tag_db_id: 3,
                state_ids: vec![219, 253, 254],
            }]
        );
        assert_ne!(extracted.source_proofs & SOURCE_PROOF_BUILD_SHA_870EBF, 0);
        assert_ne!(
            extracted.source_proofs & SOURCE_PROOF_FACTION_GROUP_870EBF,
            0
        );
        assert_ne!(
            extracted.source_proofs & SOURCE_PROOF_INTEL_NETWORK_GRAPH_870EBF,
            0
        );
        assert_ne!(
            extracted.source_proofs & SOURCE_PROOF_RELATION_ROOTS_870EBF,
            0
        );
        assert_ne!(
            extracted.source_proofs & SOURCE_PROOF_OPERATIVE_ROOT_870EBF,
            0
        );
        assert_ne!(
            extracted.source_proofs & SOURCE_PROOF_INCOMING_ACTION_QUEUE_870EBF,
            0
        );
        assert_ne!(
            extracted.source_proofs & SOURCE_PROOF_FACTION_SYSTEM_870EBF,
            0
        );
        assert_eq!(
            extracted.source_proofs
                & (SOURCE_PROOF_OPERATION_DETAIL_870EBF
                    | SOURCE_PROOF_INTEL_MAP_PERCENTAGES_870EBF
                    | SOURCE_PROOF_WAR_LIFECYCLE_DETAIL_870EBF),
            0
        );

        let report = build_domain53_payload(extracted.rows, extracted.source_proofs);
        assert!(report.ok(), "{:?}", report.errors);
        assert_ne!(
            report.gate_summary.status_flags_or & STATUS_WAR_LIFECYCLE_DETAIL_UNVALIDATED,
            0
        );
        assert_eq!(
            report.gate_summary.status_flags_or & STATUS_FACTION_GROUP_UNVALIDATED,
            0
        );
    }

    /// W1 instrument (perf_rearch_20260706 T4): the per-family wall timers are
    /// populated on a successful extraction and are internally consistent — every
    /// per-family span is measured within (and therefore <=) an outer total the
    /// test measures around the whole extraction call. Magnitudes are NOT asserted:
    /// a tiny fixture on a fast machine can legitimately record 0 micros for any
    /// family (the sub-microsecond resolution floor), so we assert PRESENCE +
    /// CONSISTENCY, never a lower bound > 0. The fixture carries relations rows
    /// (2 opinions + 2 relation flags + 1 subject), so the relations span is a
    /// real measured span whose accounting must obey the same <= total bound.
    #[test]
    fn phase_timers_are_measured_and_internally_consistent() {
        let mem = fake_domain53_memory(true, true);

        // Outer total measured around the SAME call the timers wrap. Every
        // per-family span is a strict subset of this interval, so the sum of all
        // spans — and each individual span — must be <= this total.
        let outer_started = Instant::now();
        let extracted = extract_rows_from_current_roots(&mem).expect("domain53 fake read");
        let outer_total_us = duration_micros_u64_saturating(outer_started.elapsed());

        // Presence: the fixture exercises the relations family (it emits relation
        // rows), so this extraction ran the timed relations span. On a fixture with
        // relations rows the accounting is real (not a skipped/zeroed family).
        assert!(
            !extracted.rows.opinions.is_empty()
                || !extracted.rows.relation_flags.is_empty()
                || !extracted.rows.subjects.is_empty(),
            "fixture must carry relations rows for this test to exercise the relations span"
        );

        let phase = extracted.phase_times;
        // W1.1: all 13 lane-side spans (10 families + the 3 new segments) are
        // disjoint sub-intervals of the outer interval, so each is <= the total
        // and their sum is <= the total. `roots_us` is measured in the resolve
        // site (before the family timers) but still inside the outer interval,
        // which wraps the SAME `extract_rows_from_current_roots` call, so it too
        // obeys the bound.
        let spans = [
            phase.wars_us,
            phase.faction_gate_us,
            phase.relations_us,
            phase.faction_roster_us,
            phase.incoming_actions_us,
            phase.agencies_us,
            phase.operatives_us,
            phase.operations_us,
            phase.intel_networks_us,
            phase.cryptology_us,
            phase.roots_us,
            phase.war_projection_us,
            phase.aggregation_us,
        ];

        // Each individual span is a subset of the outer interval → <= total.
        for (idx, &span) in spans.iter().enumerate() {
            assert!(
                span <= outer_total_us,
                "phase[{idx}] = {span}us exceeds outer total {outer_total_us}us"
            );
        }

        // W1.1 presence: `roots_us` is a real measured span — the fixture always
        // walks the country table in `resolve_domain53_frame_roots`, so the
        // resolve segment ran (its value can still floor to 0us on a fast host,
        // hence PRESENCE via the <= bound above, not a strict > 0).

        // The spans are disjoint sub-intervals of the outer interval, so their sum
        // cannot exceed the outer total either (untimed gaps only make the sum
        // smaller). This is the "sum of phases <= an outer total" consistency the
        // T4 spec calls for. `saturating_add` mirrors the production accumulation.
        let sum_us = spans
            .iter()
            .copied()
            .fold(0u64, |acc, span| acc.saturating_add(span));
        assert!(
            sum_us <= outer_total_us,
            "sum of phase spans {sum_us}us exceeds outer total {outer_total_us}us"
        );
    }

    /// Build a minimal single-faction CFactionSystem in fake memory. `members`
    /// is a list of (tag, is_valid_country) — an invalid member models a torn
    /// pointer / non-country object. Returns (mem, game_state, countries).
    fn fake_faction_system(
        members: &[(u32, bool)],
        member_count_override: Option<u32>,
    ) -> (FakeMemory, u64, Vec<CountryRoot>) {
        let module = 0x1400_0000_0;
        let heap = profile::DOMAIN53_HEAP_LO as u64 + 0x10_0000;
        let game_state = heap + 0x100;
        let faction_system = heap + 0x1000;
        let faction_array = heap + 0x2000;
        let faction1 = heap + 0x3000;
        let faction_members = heap + 0x4000;
        let country_base = heap + 0x5000;

        let mut mem = FakeMemory::new(module);
        mem.add_chunk(game_state, 0x500);
        mem.add_chunk(faction_system, 0x40);
        mem.add_chunk(faction_array, 0x20);
        mem.add_chunk(faction1, 0x80);
        mem.add_chunk(faction_members, 0x100);

        let mut countries = Vec::new();
        for (idx, (tag, is_country)) in members.iter().enumerate() {
            let cptr = country_base + (idx as u64) * 0x200;
            mem.add_chunk(cptr, 0x100);
            if *is_country {
                mem.write_u64(cptr, module + profile::DOMAIN53_COUNTRY_VTABLE_RVA as u64);
                mem.write_u32(cptr + profile::DOMAIN53_OBJ_TAG_DB_ID as u64, *tag);
                countries.push(CountryRoot {
                    tag_db_id: *tag,
                    country_ptr: cptr,
                });
            } else {
                // Wrong vtable => not a validated CCountry.
                mem.write_u64(cptr, module + 0x00DE_AD00);
            }
            mem.write_u64(faction_members + (idx as u64) * 8, cptr);
        }

        mem.write_u64(
            game_state + profile::DOMAIN53_CGAMESTATE_FACTION_SYSTEM as u64,
            faction_system,
        );
        mem.write_u64(
            faction_system + profile::DOMAIN53_FACTION_ARRAY as u64,
            faction_array,
        );
        mem.write_u64(
            faction_system + profile::DOMAIN53_FACTION_COUNT as u64,
            (1u64 << 32) | 1u64,
        );
        mem.write_u64(faction_array, faction1);
        mem.write_u64(
            faction1 + profile::DOMAIN53_FACTION_ID as u64,
            (1u64 << 32) | u64::from(profile::DOMAIN53_FACTION_TYPE_EXPECTED),
        );
        mem.write_inline_string(faction1 + profile::DOMAIN53_FACTION_NAME as u64, "Axis");
        mem.write_u64(
            faction1 + profile::DOMAIN53_FACTION_MEMBERS as u64,
            faction_members,
        );
        mem.write_u32(
            faction1 + profile::DOMAIN53_FACTION_MEMBERS_SIZE as u64,
            member_count_override.unwrap_or(members.len() as u32),
        );
        (mem, game_state, countries)
    }

    #[test]
    fn faction_reader_fails_closed_when_a_member_is_not_a_country() {
        // Second member is a non-country object -> the whole roster is torn -> [].
        let (mem, game_state, countries) = fake_faction_system(&[(1, true), (2, false)], None);
        let rows =
            read_faction_rows(&mem, mem.module_base, game_state, &countries).expect("faction read");
        assert!(rows.is_empty());
    }

    #[test]
    fn faction_reader_fails_closed_on_oversized_member_count() {
        // member_count tears UPWARD to 5 while only 2 member slots are populated
        // (slots 2..5 read 0 => non-heap). The reader must NOT emit a partial
        // faction: any unreadable member => whole roster empty. (A DOWNWARD tear
        // that stays internally consistent is indistinguishable from a genuinely
        // smaller faction without an external oracle; the orchestrator's melt
        // member-count check — {18,31,3,8,10,1} — is the defense there.)
        let (mem, game_state, countries) = fake_faction_system(&[(1, true), (2, true)], Some(5));
        let rows =
            read_faction_rows(&mem, mem.module_base, game_state, &countries).expect("faction read");
        assert!(rows.is_empty());
    }

    #[test]
    fn faction_reader_emits_clean_roster() {
        let (mem, game_state, countries) =
            fake_faction_system(&[(1, true), (2, true), (3, true)], None);
        let rows =
            read_faction_rows(&mem, mem.module_base, game_state, &countries).expect("faction read");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].faction_id, 1);
        assert_eq!(rows[0].leader_tag_db_id, 1);
        assert_eq!(rows[0].member_tag_db_ids, vec![1, 2, 3]);
    }

    #[test]
    fn discovery_extract_matches_uncached_fake_memory() {
        let mem = fake_domain53_memory(true, true);
        let discovery = discover_domain53_heap_hits(&mem).expect("domain53 discovery");
        let mut cached = extract_rows_from_current_roots_with_discovery(&mem, &discovery)
            .expect("cached domain53 extract");
        let mut uncached =
            extract_rows_from_current_roots(&mem).expect("uncached domain53 extract");

        // W1 instrument (perf_rearch_20260706 T4): `phase_times` is observational
        // wall-clock noise — two runs of the same extraction legitimately record
        // different micros — so it is NOT part of the data equivalence this test
        // asserts. Neutralize it on both sides before the full-struct compare; the
        // rows/proofs/counts equivalence (the real invariant) is unchanged.
        cached.phase_times = Domain53PhaseTimes::default();
        uncached.phase_times = Domain53PhaseTimes::default();

        assert_eq!(cached, uncached);
    }

    /// P4b: the off-frame validated-address war path yields the SAME war rows as
    /// the raw inline path, and per-frame re-reads casualties LIVE (not frozen).
    #[test]
    fn validated_war_addrs_match_raw_and_reread_casualties_fresh() {
        let mut mem = fake_domain53_memory(true, true);
        let discovery = discover_domain53_heap_hits(&mem).expect("domain53 discovery");

        // Off-frame validation collapses the raw war-date hits to the one real
        // war-row address (row_addr = hit - WAR_DATE_VTABLE).
        let validated =
            domain53_validate_war_row_addrs(&mem, &discovery.war_hits[0], &discovery.war_hits[1])
                .expect("validate war addrs");
        assert_eq!(validated.len(), 1);

        // Same war rows as the raw path (byte-identical war family).
        let raw = extract_rows_from_current_roots_with_discovery(&mem, &discovery).expect("raw");
        let via_addrs =
            extract_rows_from_current_roots_with_validated_war_addrs(&mem, &discovery, &validated)
                .expect("validated");
        assert_eq!(via_addrs.rows.wars, raw.rows.wars);
        assert_eq!(via_addrs.rows.wars.len(), 1);
        assert_eq!(via_addrs.rows.wars[0].first_casualties, 100);
        assert_eq!(via_addrs.rows.wars[0].second_casualties, 200);
        assert_eq!(via_addrs.skipped_rows, 0);

        // CASUALTIES NOT FROZEN: mutate the live row AFTER validation, re-extract
        // from the SAME validated addresses → the fresh values appear.
        let war_row = validated[0];
        mem.write_i32(
            war_row + profile::DOMAIN53_WAR_FIRST_CASUALTIES as u64,
            4242,
        );
        mem.write_i32(
            war_row + profile::DOMAIN53_WAR_SECOND_CASUALTIES as u64,
            9999,
        );
        let refreshed =
            extract_rows_from_current_roots_with_validated_war_addrs(&mem, &discovery, &validated)
                .expect("refreshed");
        assert_eq!(refreshed.rows.wars.len(), 1);
        assert_eq!(refreshed.rows.wars[0].first_casualties, 4242);
        assert_eq!(refreshed.rows.wars[0].second_casualties, 9999);

        // A validated address whose war-date vtable is gone (war ended / slot
        // reused) is dropped by per-frame revalidation, never emitted as a bogus
        // row (ABA-safe).
        mem.write_u64(war_row + profile::DOMAIN53_WAR_DATE_VTABLE as u64, 0);
        let ended =
            extract_rows_from_current_roots_with_validated_war_addrs(&mem, &discovery, &validated)
                .expect("war-ended extract");
        assert!(ended.rows.wars.is_empty());
    }

    #[test]
    fn cached_operative_stale_hit_drops_only_that_row() {
        let mut mem = fake_domain53_memory(true, true);
        let discovery = discover_domain53_heap_hits(&mem).expect("domain53 discovery");
        assert_eq!(discovery.operative_hits.len(), 1);
        mem.write_u64(discovery.operative_hits[0], 0);

        let extracted = extract_rows_from_current_roots_with_discovery(&mem, &discovery)
            .expect("stale operative hit below degradation threshold");

        assert!(extracted.rows.operatives.is_empty());
        assert_eq!(extracted.rows.wars.len(), 1);
        assert_eq!(extracted.skipped_rows, 0);
        assert_eq!(extracted.stale_dropped_total, 1);
        assert_ne!(
            extracted.source_proofs & SOURCE_PROOF_OPERATIVE_ROOT_870EBF,
            0
        );
    }

    #[test]
    fn cached_operative_stale_majority_fails_closed() {
        let mut mem = fake_domain53_memory(true, true);
        for index in 0..7 {
            add_fake_operative(&mut mem, index);
        }
        let discovery = discover_domain53_heap_hits(&mem).expect("domain53 discovery");
        assert_eq!(discovery.operative_hits.len(), 8);
        for hit in discovery.operative_hits.iter().take(5) {
            mem.write_u64(*hit, 0);
        }

        let err = extract_rows_from_current_roots_with_discovery(&mem, &discovery)
            .expect_err("stale operative majority must fail closed");

        assert_eq!(err, ExtractError::InvalidRoot(DOMAIN53_DISCOVERY_DEGRADED));
    }

    /// Add a COperationInstance to the scene: vtable, handle {id, type=4713},
    /// def ptr (COperation vtable when `def_ok`), country ptr @+0x50. Returns
    /// the instance address (a discovery hit). `country_ptr` may be 0 for the
    /// absent-country case.
    fn add_fake_operation_instance(
        mem: &mut FakeMemory,
        index: u32,
        instance_id: u32,
        def_ok: bool,
        country_ptr: u64,
    ) -> u64 {
        let heap = fake_heap_base();
        let instance = heap + 0x50_0000 + u64::from(index) * 0x2000;
        let def = instance + 0x1000;
        mem.add_chunk(instance, 0x100);
        mem.add_chunk(def, 0x10);
        mem.add_range(instance, 0x100);
        mem.write_u64(
            instance,
            mem.module_base + profile::DOMAIN53_OPERATION_INSTANCE_VTABLE_RVA as u64,
        );
        // Handle @+0x08: id = HI32, type = LO32 = 4713.
        let handle = (u64::from(instance_id) << 32)
            | u64::from(profile::DOMAIN53_OPERATION_INSTANCE_HANDLE_TYPE);
        mem.write_u64(
            instance + profile::DOMAIN53_OPERATION_INSTANCE_HANDLE as u64,
            handle,
        );
        // Def ptr @+0x48 -> COperation vtable (only when def_ok).
        mem.write_u64(
            instance + profile::DOMAIN53_OPERATION_INSTANCE_DEF as u64,
            def,
        );
        let def_vtable = if def_ok {
            mem.module_base + profile::DOMAIN53_OPERATION_DEF_VTABLE_RVA as u64
        } else {
            mem.module_base + 0x1234
        };
        mem.write_u64(def, def_vtable);
        // Country ptr @+0x50.
        mem.write_u64(
            instance + profile::DOMAIN53_OPERATION_INSTANCE_TARGET_COUNTRY as u64,
            country_ptr,
        );
        // +0x10 owner is REJECTED: write garbage to prove it is never read.
        mem.write_u64(instance + 0x10, 0x593B_0001);
        instance
    }

    #[test]
    fn read_operations_validates_instances_and_resolves_country() {
        let mut mem = fake_domain53_memory(true, true);
        let module = mem.module_base;
        // country3 (tag 3) address in the scene: heap + 0x5_0000.
        let country3 = fake_heap_base() + 0x5_0000;
        // Instance A: valid, def ok, target country 3.
        let inst_a = add_fake_operation_instance(&mut mem, 0, 129_014, true, country3);
        // Instance B: valid, def NOT ok, no country.
        let inst_b = add_fake_operation_instance(&mut mem, 1, 128_904, false, 0);
        // Instance C: wrong handle type -> dropped as stale.
        let inst_c = add_fake_operation_instance(&mut mem, 2, 7, true, country3);
        mem.write_u64(
            inst_c + profile::DOMAIN53_OPERATION_INSTANCE_HANDLE as u64,
            (7u64 << 32) | 9999, // type != 4713
        );

        let discovery = discover_domain53_heap_hits(&mem).expect("domain53 discovery");
        assert!(discovery.operation_instance_hits.contains(&inst_a));
        assert!(discovery.operation_instance_hits.contains(&inst_b));
        assert!(discovery.operation_instance_hits.contains(&inst_c));

        let countries =
            resolve_country_roots(&mem, module, fake_heap_base()).expect("country roots");
        let (rows, drops) =
            read_operations(&mem, module, &countries, &discovery.operation_instance_hits)
                .expect("read_operations");

        // C dropped (bad handle type) — that is candidate FILTERING (the vtable
        // matched but it is not a valid instance), NOT liveness staleness. A+C
        // fix: it must be counted as `filtered`, never `stale_liveness` (the
        // latter is what nuked the whole frame). A and B kept, sorted by id.
        assert_eq!(drops.stale_liveness, 0);
        assert_eq!(drops.filtered, 1);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].instance_id, 128_904);
        assert!(!rows[0].def_ref_ok);
        assert_eq!(rows[0].associated_country_tag_db_id, None);
        assert_eq!(rows[1].instance_id, 129_014);
        assert!(rows[1].def_ref_ok);
        assert_eq!(rows[1].associated_country_tag_db_id, Some(3));
    }

    #[test]
    fn operations_never_nuke_the_frame_operatives_are_the_systemic_gate() {
        // A+C fix (2026-07-07, D53_ROOT_CAUSE.md): a stale-MAJORITY of operations
        // must NOT degrade the whole d53 frame. Only the OPERATIVE family gates
        // the frame (it's the reliable shared-sweep-death signal); operations are
        // row-local fail-closed and surface health via `operation_degraded()` as
        // a DIAGNOSTIC, never a nuke.

        // Operations stale-majority (5 of 8), operatives clean: NOT
        // frame-degraded (operations are out of the gate), but operation_degraded
        // flags it for observability.
        let ops_majority = Domain53DiscoveryStaleness {
            operative_total: 8,
            operative_stale_dropped: 0,
            war_total: 0,
            war_stale_dropped: 0,
            operation_total: 8,
            operation_stale_dropped: 5,
            operation_filtered: 0,
        };
        assert!(
            !ops_majority.frame_degraded(),
            "an operation stale majority must NOT nuke the whole frame"
        );
        assert!(
            ops_majority.operation_degraded(),
            "operation health must still be visible via the diagnostic gate"
        );

        // Operation FILTERING (false positives / dedup) is not staleness at all:
        // even an all-filtered operation set must neither frame-degrade NOR
        // operation-degrade, and must not inflate the visible stale total.
        let ops_all_filtered = Domain53DiscoveryStaleness {
            operative_total: 8,
            operative_stale_dropped: 0,
            war_total: 0,
            war_stale_dropped: 0,
            operation_total: 8,
            operation_stale_dropped: 0,
            operation_filtered: 8,
        };
        assert!(!ops_all_filtered.frame_degraded());
        assert!(
            !ops_all_filtered.operation_degraded(),
            "candidate filtering is not staleness"
        );
        assert_eq!(
            ops_all_filtered.stale_dropped_total(),
            0,
            "filtering must not count toward the visible stale total"
        );

        // Operative stale-majority alone still degrades the frame (operations
        // clean) — operatives remain the systemic guard.
        let operatives_majority = Domain53DiscoveryStaleness {
            operative_total: 8,
            operative_stale_dropped: 5,
            war_total: 0,
            war_stale_dropped: 0,
            operation_total: 8,
            operation_stale_dropped: 0,
            operation_filtered: 0,
        };
        assert!(operatives_majority.frame_degraded());
    }

    #[test]
    fn war_staleness_never_contributes_to_the_whole_frame_gate() {
        // P4d: the war family is deliberately OUT of the whole-frame gate. A
        // war stale-MAJORITY with clean operatives/operations must NOT
        // frame-degrade (it downgrades the war family instead); it MUST show up
        // on the separate `war_degraded()` gate.
        let war_only = Domain53DiscoveryStaleness {
            operative_total: 8,
            operative_stale_dropped: 0,
            war_total: 8,
            war_stale_dropped: 8,
            operation_total: 8,
            operation_stale_dropped: 0,
            operation_filtered: 0,
        };
        assert!(
            !war_only.frame_degraded(),
            "a war stale majority must never doom the whole frame"
        );
        assert!(
            war_only.war_degraded(),
            "a war stale majority must trip the war-only downgrade gate"
        );

        // Conversely, an operative majority trips the frame gate but NOT the
        // war gate — the two are fully independent.
        let op_only = Domain53DiscoveryStaleness {
            operative_total: 8,
            operative_stale_dropped: 5,
            war_total: 8,
            war_stale_dropped: 0,
            operation_total: 8,
            operation_stale_dropped: 0,
            operation_filtered: 0,
        };
        assert!(op_only.frame_degraded());
        assert!(!op_only.war_degraded());
    }

    #[test]
    fn cached_operation_stale_majority_still_emits_the_rest_of_d53() {
        // A+C fix (D53_ROOT_CAUSE.md): a stale-MAJORITY of live operation hits
        // must NOT nuke the whole d53 frame. The operatives, relations, agencies,
        // etc. are unrelated and clean, so the frame EMITS — dropping only the
        // dead operation rows and surfacing the split on `family_counts`. This is
        // the exact behavior change that restores d53 warm on a live speed-5 game
        // (operations went ~majority "stale" every frame, nuking everything).
        let mut mem = fake_domain53_memory(true, true);
        let country3 = fake_heap_base() + 0x5_0000;
        for index in 0..8 {
            add_fake_operation_instance(&mut mem, index, 100 + index, true, country3);
        }
        let discovery = discover_domain53_heap_hits(&mem).expect("domain53 discovery");
        assert_eq!(discovery.operation_instance_hits.len(), 8);
        assert_eq!(discovery.operative_hits.len(), 1);
        // GENUINELY stale 5 of 8 operation hits (zero the vtable -> liveness
        // failure, a majority); leave the operative clean.
        for hit in discovery.operation_instance_hits.iter().take(5) {
            mem.write_u64(*hit, 0);
        }

        let extracted = extract_rows_from_current_roots_with_discovery(&mem, &discovery)
            .expect("operation stale majority must NOT fail the whole frame");
        // The rest of d53 still emits.
        assert_eq!(extracted.rows.operatives.len(), 1);
        // Only the 3 live operations survive; the 5 dead ones are dropped.
        assert_eq!(extracted.rows.operation_instances.len(), 3);
        // The split is visible for observability (candidates = emitted + stale).
        assert_eq!(extracted.family_counts.operation_candidates, 8);
        assert_eq!(extracted.family_counts.operation_emitted, 3);
        assert_eq!(extracted.family_counts.operation_stale_liveness, 5);
        assert_eq!(extracted.family_counts.operation_filtered, 0);
    }

    #[test]
    fn cached_operation_stale_hit_below_majority_does_not_degrade_operatives() {
        // A single stale operation hit (below the majority threshold) drops
        // only that operation row and does NOT fail the frame or the operative
        // family — the operatives extract cleanly.
        let mut mem = fake_domain53_memory(true, true);
        let country3 = fake_heap_base() + 0x5_0000;
        for index in 0..8 {
            add_fake_operation_instance(&mut mem, index, 100 + index, true, country3);
        }
        let discovery = discover_domain53_heap_hits(&mem).expect("domain53 discovery");
        // Stale exactly 1 of 8 operation hits (well below majority).
        mem.write_u64(discovery.operation_instance_hits[0], 0);

        let extracted = extract_rows_from_current_roots_with_discovery(&mem, &discovery)
            .expect("single stale operation hit is below degradation threshold");
        // Operatives untouched.
        assert_eq!(extracted.rows.operatives.len(), 1);
        // 7 of 8 operations survived.
        assert_eq!(extracted.rows.operation_instances.len(), 7);
        assert_eq!(extracted.stale_dropped_total, 1);
    }

    #[test]
    fn cached_war_stale_hit_drops_only_that_row() {
        let mut mem = fake_domain53_memory(true, true);
        let discovery = discover_domain53_heap_hits(&mem).expect("domain53 discovery");
        assert_eq!(total_war_hits(&discovery), 1);
        let hit = discovery
            .war_hits
            .iter()
            .find_map(|hits| hits.first().copied())
            .expect("fake war hit");
        mem.write_u64(hit, 0);

        let extracted = extract_rows_from_current_roots_with_discovery(&mem, &discovery)
            .expect("stale war hit below degradation threshold");

        assert!(extracted.rows.wars.is_empty());
        assert_eq!(extracted.rows.operatives.len(), 1);
        assert_eq!(extracted.skipped_rows, 0);
        assert_eq!(extracted.stale_dropped_total, 1);
        assert_ne!(extracted.source_proofs & SOURCE_PROOF_WAR_ROOT_870EBF, 0);
    }

    #[test]
    fn cached_war_stale_majority_downgrades_war_family_but_still_emits() {
        // P4d fix: a stale-MAJORITY of the war set no longer nukes the WHOLE
        // d53 frame. War is downgraded — war rows dropped AND
        // `CountryDiplomacyRow`s OMITTED (a zeroed war projection would be false
        // "no wars" data in the replay fold, which publishes country war fields
        // unconditionally) with `SOURCE_PROOF_WAR_ROOT_870EBF` withheld ->
        // STATUS_WAR_ROOT_UNVALIDATED — while operatives still emit. A few
        // churning wars at speed 5 must NOT throw away the operative data.
        let mut mem = fake_domain53_memory(true, true);
        for index in 0..8 {
            add_fake_war(&mut mem, index);
        }
        let discovery = discover_domain53_heap_hits(&mem).expect("domain53 discovery");
        assert_eq!(total_war_hits(&discovery), 9);
        let mut corrupted = 0usize;
        for hit in discovery
            .war_hits
            .iter()
            .flat_map(|hits| hits.iter())
            .take(5)
        {
            mem.write_u64(*hit, 0);
            corrupted += 1;
        }
        assert_eq!(corrupted, 5);

        let extracted = extract_rows_from_current_roots_with_discovery(&mem, &discovery)
            .expect("war stale majority downgrades the war family, does not fail the frame");

        // War family downgraded: no war rows, no war-root proof.
        assert!(extracted.rows.wars.is_empty(), "war rows must be dropped");
        assert_eq!(
            extracted.source_proofs & SOURCE_PROOF_WAR_ROOT_870EBF,
            0,
            "war-root proof must be withheld on a stale war majority"
        );
        // NO country diplomacy rows are emitted — an omitted row is honest "war
        // state unknown", a zeroed row would be false data in the fold.
        assert!(
            extracted.rows.countries.is_empty(),
            "country diplomacy rows must be omitted when war is downgraded"
        );
        // ...but the faction-group signal survives via state events (no war
        // fields), so faction data is not lost to war churn.
        assert!(
            extracted
                .rows
                .state_events
                .iter()
                .any(|e| e.source_kind == STATE_EVENT_SOURCE_COUNTRY_FACTION_GROUP),
            "faction-group state events must still emit when war is downgraded"
        );
        // Operatives untouched — this is the whole point of the fix.
        assert_eq!(extracted.rows.operatives.len(), 1);
        assert_eq!(extracted.skipped_rows, 0);
        // The stale war addresses are still counted in the visible stale total.
        assert_eq!(extracted.stale_dropped_total, 5);

        // The emitted payload validates and carries STATUS_WAR_ROOT_UNVALIDATED
        // (NOT a whole-lane failure).
        let report = build_domain53_payload(extracted.rows, extracted.source_proofs);
        assert!(report.ok(), "{:?}", report.errors);
        assert_ne!(
            report.gate_summary.status_flags_or & STATUS_WAR_ROOT_UNVALIDATED,
            0,
            "downgraded war family must badge STATUS_WAR_ROOT_UNVALIDATED"
        );
    }

    #[test]
    fn validated_war_addrs_stale_majority_downgrades_war_only_operatives_survive() {
        // The LIVE (P4b) path: war rows come from a SMALL validated-address set
        // re-read every frame. At speed 5 several of those addresses go stale
        // (retired/reused war). Before the P4d fix this tripped the whole-frame
        // majority gate and emitted SOURCE_UNAVAILABLE every warm frame. Now it
        // downgrades ONLY the war family.
        let mut mem = fake_domain53_memory(true, true);
        // Build a validated war-address set of 8 (row_addr = hit - vtable off).
        let mut addrs = Vec::new();
        for index in 0..8 {
            let war = add_fake_war(&mut mem, index);
            addrs.push(war);
        }
        // The operative discovery still comes from the on-frame cached hits.
        let discovery = discover_domain53_heap_hits(&mem).expect("domain53 discovery");
        // Stale 5 of 8 validated war addresses by clobbering their date vtable
        // slot (freed/reused pool slot -> stale_dropped on re-read).
        for war in addrs.iter().take(5) {
            mem.write_u64(war + profile::DOMAIN53_WAR_DATE_VTABLE as u64, 0);
        }

        let extracted =
            extract_rows_from_current_roots_with_validated_war_addrs(&mem, &discovery, &addrs)
                .expect("validated-addr war stale majority must downgrade war, not fail the frame");

        assert!(extracted.rows.wars.is_empty());
        assert_eq!(extracted.source_proofs & SOURCE_PROOF_WAR_ROOT_870EBF, 0);
        // Country diplomacy rows OMITTED (no false zeroed war projection in the
        // fold); faction-group signal preserved via state events.
        assert!(extracted.rows.countries.is_empty());
        assert!(extracted
            .rows
            .state_events
            .iter()
            .any(|e| e.source_kind == STATE_EVENT_SOURCE_COUNTRY_FACTION_GROUP));
        assert_eq!(extracted.rows.operatives.len(), 1);
        assert_eq!(extracted.stale_dropped_total, 5);

        let report = build_domain53_payload(extracted.rows, extracted.source_proofs);
        assert!(report.ok(), "{:?}", report.errors);
        assert_ne!(
            report.gate_summary.status_flags_or & STATUS_WAR_ROOT_UNVALIDATED,
            0
        );
    }

    #[test]
    fn validated_war_addrs_below_majority_keeps_war_family_validated() {
        // Boundary / churn tolerance: a MINORITY of stale validated war
        // addresses (3 of 8) keeps the war family fully validated — the
        // surviving wars project into country rows and the war-root proof is
        // set. Only a majority downgrades.
        let mut mem = fake_domain53_memory(true, true);
        let mut addrs = Vec::new();
        for index in 0..8 {
            addrs.push(add_fake_war(&mut mem, index));
        }
        let discovery = discover_domain53_heap_hits(&mem).expect("domain53 discovery");
        // Stale exactly 3 of 8 (below the stale*2 > total majority threshold).
        for war in addrs.iter().take(3) {
            mem.write_u64(war + profile::DOMAIN53_WAR_DATE_VTABLE as u64, 0);
        }

        let extracted =
            extract_rows_from_current_roots_with_validated_war_addrs(&mem, &discovery, &addrs)
                .expect("minority-stale war set stays validated");

        // 5 of 8 wars survived and the war-root proof is present.
        assert_eq!(extracted.rows.wars.len(), 5);
        assert_ne!(extracted.source_proofs & SOURCE_PROOF_WAR_ROOT_870EBF, 0);
        assert_eq!(extracted.stale_dropped_total, 3);
        // At least one country carries a nonzero at-war projection.
        assert!(extracted
            .rows
            .countries
            .iter()
            .any(|country| country.at_war_count != 0));

        let report = build_domain53_payload(extracted.rows, extracted.source_proofs);
        assert!(report.ok(), "{:?}", report.errors);
        assert_eq!(
            report.gate_summary.status_flags_or & STATUS_WAR_ROOT_UNVALIDATED,
            0
        );
    }

    #[test]
    fn war_boundary_exactly_at_majority_threshold_downgrades() {
        // Threshold boundary for `stale_fraction_degraded` on the war family:
        // total=8, stale*2 > total means stale >= 5 degrades; stale == 4 does
        // NOT. Verify the exact tipping point on the validated-addr path.
        let build = |stale_count: usize| {
            let mut mem = fake_domain53_memory(true, true);
            let mut addrs = Vec::new();
            for index in 0..8 {
                addrs.push(add_fake_war(&mut mem, index));
            }
            let discovery = discover_domain53_heap_hits(&mem).expect("domain53 discovery");
            for war in addrs.iter().take(stale_count) {
                mem.write_u64(war + profile::DOMAIN53_WAR_DATE_VTABLE as u64, 0);
            }
            extract_rows_from_current_roots_with_validated_war_addrs(&mem, &discovery, &addrs)
                .expect("frame always extracts (war only downgrades)")
        };

        // stale == 4 of 8: NOT a majority -> war stays validated.
        let at_four = build(4);
        assert_ne!(at_four.source_proofs & SOURCE_PROOF_WAR_ROOT_870EBF, 0);
        assert_eq!(at_four.rows.wars.len(), 4);

        // stale == 5 of 8: majority -> war downgraded.
        let at_five = build(5);
        assert_eq!(at_five.source_proofs & SOURCE_PROOF_WAR_ROOT_870EBF, 0);
        assert!(at_five.rows.wars.is_empty());
    }

    #[test]
    fn war_below_absolute_floor_never_downgrades_even_if_fraction_high() {
        // The `total >= 8` absolute floor: with a tiny validated war set the
        // fraction test is disabled, so a small set that is entirely stale is
        // NOT treated as systemic degradation (it drops row-by-row instead).
        // 3 of 4 stale is a 75% fraction but total < 8 -> war stays validated.
        let mut mem = fake_domain53_memory(true, true);
        let mut addrs = Vec::new();
        for index in 0..4 {
            addrs.push(add_fake_war(&mut mem, index));
        }
        let discovery = discover_domain53_heap_hits(&mem).expect("domain53 discovery");
        for war in addrs.iter().take(3) {
            mem.write_u64(war + profile::DOMAIN53_WAR_DATE_VTABLE as u64, 0);
        }

        let extracted =
            extract_rows_from_current_roots_with_validated_war_addrs(&mem, &discovery, &addrs)
                .expect("below the absolute floor the war family is never downgraded");

        // 1 war survived, war-root proof present (row-by-row fail-closed, not a
        // systemic downgrade).
        assert_eq!(extracted.rows.wars.len(), 1);
        assert_ne!(extracted.source_proofs & SOURCE_PROOF_WAR_ROOT_870EBF, 0);
        assert_eq!(extracted.stale_dropped_total, 3);
    }

    #[test]
    fn operative_stale_majority_still_fails_whole_frame_when_war_downgrades() {
        // Cross-family separation under the new regime: a stale-MAJORITY of the
        // OPERATIVE family still fails the WHOLE frame closed (it has no
        // per-family unvalidated status), EVEN when the war family would only
        // downgrade. This models a dead/ancient sweep where operatives are
        // systemically stale too — the lane must still emit SOURCE_UNAVAILABLE.
        let mut mem = fake_domain53_memory(true, true);
        for index in 0..7 {
            add_fake_operative(&mut mem, index);
        }
        let mut addrs = Vec::new();
        for index in 0..8 {
            addrs.push(add_fake_war(&mut mem, index));
        }
        let discovery = discover_domain53_heap_hits(&mem).expect("domain53 discovery");
        assert_eq!(discovery.operative_hits.len(), 8);
        // Stale 5 of 8 operatives (a majority) AND 5 of 8 validated wars.
        for hit in discovery.operative_hits.iter().take(5) {
            mem.write_u64(*hit, 0);
        }
        for war in addrs.iter().take(5) {
            mem.write_u64(war + profile::DOMAIN53_WAR_DATE_VTABLE as u64, 0);
        }

        let err =
            extract_rows_from_current_roots_with_validated_war_addrs(&mem, &discovery, &addrs)
                .expect_err("operative stale majority must still fail the whole frame closed");
        assert_eq!(err, ExtractError::InvalidRoot(DOMAIN53_DISCOVERY_DEGRADED));
    }

    #[test]
    fn reader_fails_closed_before_build_proof_on_wrong_country_rtti() {
        let mut mem = fake_domain53_memory(true, true);
        mem.write_rtti_name(TEST_COUNTRY_TYPE_DESCRIPTOR_RVA, ".?AVCWrongCountry@@");

        let err = extract_rows_from_current_roots(&mem).expect_err("wrong RTTI must fail closed");
        assert!(matches!(err, ExtractError::Validation(_)), "{err:?}");

        let report = build_domain53_payload(
            proof_fixture_rows(),
            CURRENT_VALIDATED_SOURCE_PROOFS & !SOURCE_PROOF_BUILD_SHA_870EBF,
        );
        assert!(!report.ok());
        assert!(report
            .errors
            .iter()
            .any(|error| error.contains("SOURCE_PROOF_BUILD_SHA_870EBF")));
    }

    #[test]
    fn reader_fails_closed_before_build_proof_on_garbled_agency_rtti() {
        let mut mem = fake_domain53_memory(true, true);
        mem.write_rtti_name(TEST_AGENCY_TYPE_DESCRIPTOR_RVA, "not_rtti");

        let err = extract_rows_from_current_roots(&mem).expect_err("garbled RTTI must fail closed");
        assert!(matches!(
            err,
            ExtractError::InvalidRoot("domain53_rtti_name_invalid")
        ));
    }

    #[test]
    fn reader_omits_faction_groups_when_structural_gate_fails() {
        let mut mem = fake_domain53_memory(true, true);
        let heap = profile::DOMAIN53_HEAP_LO as u64 + 0x10_0000;
        let country1 = heap + 0x3_0000;
        mem.write_i32(
            country1 + profile::DOMAIN53_COUNTRY_FACTION_GROUP as u64,
            -1,
        );

        let extracted =
            extract_rows_from_current_roots(&mem).expect("other slices remain readable");

        assert_eq!(
            extracted.source_proofs & SOURCE_PROOF_FACTION_GROUP_870EBF,
            0
        );
        assert!(extracted.rows.countries.is_empty());
        assert!(!extracted
            .rows
            .state_events
            .iter()
            .any(|row| row.source_kind == STATE_EVENT_SOURCE_COUNTRY_FACTION_GROUP));

        let report = build_domain53_payload(extracted.rows, extracted.source_proofs);
        assert!(report.ok(), "{:?}", report.errors);
        assert_ne!(
            report.gate_summary.status_flags_or & STATUS_FACTION_GROUP_UNVALIDATED,
            0
        );
    }

    #[test]
    fn reader_fails_closed_on_worldwide_zero_relations() {
        let mem = fake_domain53_memory(false, true);
        let err = extract_rows_from_current_roots(&mem).expect_err("relations must fail closed");
        assert_eq!(
            err,
            ExtractError::InvalidRoot("domain53_relations_empty_implausible")
        );
    }

    #[test]
    fn reader_accepts_zero_operatives_as_resolved_empty() {
        let mem = fake_domain53_memory(true, false);
        let extracted = extract_rows_from_current_roots(&mem).expect("zero operative read");

        assert!(extracted.rows.operatives.is_empty());
        assert_ne!(
            extracted.source_proofs & SOURCE_PROOF_OPERATIVE_ROOT_870EBF,
            0
        );
        let report = build_domain53_payload(extracted.rows, extracted.source_proofs);
        assert!(report.ok(), "{:?}", report.errors);
    }

    #[test]
    fn current_validated_sources_emit_badged_domain53_payload() {
        let report = build_domain53_payload(proof_fixture_rows(), CURRENT_VALIDATED_SOURCE_PROOFS);

        assert!(report.ok(), "{:?}", report.errors);
        assert_eq!(report.domain_id, 53);
        assert_eq!(report.gate_summary.data_rows_total, 18);
        assert_eq!(
            report
                .gate_summary
                .data_units_total
                .get("agency_capacity_cluster_rows")
                .copied(),
            Some(1)
        );
        assert_eq!(
            report
                .gate_summary
                .data_units_total
                .get("operative_lifecycle_rows")
                .copied(),
            Some(3)
        );
        assert_eq!(
            report
                .gate_summary
                .data_units_total
                .get("compact_state_event_rows")
                .copied(),
            Some(8)
        );
        assert_eq!(
            report
                .gate_summary
                .data_units_total
                .get("compact_state_event_relation_opinion_rows")
                .copied(),
            Some(1)
        );
        assert_eq!(
            report
                .gate_summary
                .data_units_total
                .get("compact_state_event_country_faction_group_rows")
                .copied(),
            Some(1)
        );
        assert_eq!(
            report.gate_summary.status_flags_or & STATUS_WAR_ROOT_UNVALIDATED,
            0
        );
        assert_eq!(
            report.gate_summary.status_flags_or & STATUS_INCOMING_ACTION_QUEUE_UNVALIDATED,
            0
        );
        assert_eq!(
            report.gate_summary.status_flags_or & STATUS_INTEL_NETWORK_GRAPH_UNVALIDATED,
            0
        );
        assert_eq!(
            report
                .gate_summary
                .data_units_total
                .get("incoming_action_queue_rows")
                .copied(),
            Some(2)
        );
        assert_eq!(
            report
                .gate_summary
                .data_units_total
                .get("intel_network_graph_rows")
                .copied(),
            Some(2)
        );
        assert_eq!(
            report
                .gate_summary
                .data_units_total
                .get("intel_network_graph_state_rows")
                .copied(),
            Some(17)
        );
        assert_eq!(
            report
                .gate_summary
                .data_units_total
                .get("incoming_action_queue_recipient_rows")
                .copied(),
            Some(2)
        );
        assert_ne!(
            report.gate_summary.status_flags_or & STATUS_WAR_LIFECYCLE_DETAIL_UNVALIDATED,
            0
        );
        assert_ne!(
            report.gate_summary.status_flags_or & STATUS_OPERATIONS_DETAIL_RESISTED,
            0
        );
        assert_ne!(
            report.gate_summary.status_flags_or & STATUS_INTEL_MAP_RESISTED,
            0
        );
        assert_eq!(
            report
                .gate_summary
                .data_units_total
                .get("war_relation_rows")
                .copied(),
            Some(0)
        );
    }

    #[test]
    fn payload_phase_times_populated_and_bounded_by_outer_build() {
        // W1.2/W2: the sub-phase timers split the single collector
        // `seg_payload_us`. Post-W2 the production path always runs encode +
        // summarize (the encode-side gate), while decode-gate now runs ONLY on the
        // 1/N sampled cross-check — so this test forces sampling ON to exercise all
        // spans, and asserts their sum is <= the wall of the outer build call (the
        // outer call also pays the report struct assembly, so sum + remainder ==
        // outer). Lane-level counterpart of the collector's seg-split accounting.
        let outer_started = Instant::now();
        let report = build_domain53_payload_sampled(
            proof_fixture_rows(),
            CURRENT_VALIDATED_SOURCE_PROOFS,
            0,
            true,
        );
        let outer_us = duration_micros_u64_saturating(outer_started.elapsed());

        assert!(report.ok(), "{:?}", report.errors);

        let pt = report.payload_phase_times;
        // encode + summarize + (forced) decode-gate always do real work here.
        assert!(!report.payload.is_empty(), "fixture must produce a payload");
        assert!(pt.encode_us > 0, "encode phase must be timed");
        assert!(pt.summary_us > 0, "encode-side summary phase must be timed");
        assert!(
            pt.decode_gate_us > 0,
            "sampled decode-gate phase must be timed"
        );

        // The sub-phases are disjoint spans inside the build; their sum can never
        // exceed the wall of the whole build call (which also includes the report
        // struct assembly). A tiny remainder is expected and acceptable.
        let phase_sum =
            pt.snapshot_us + pt.validation_us + pt.encode_us + pt.summary_us + pt.decode_gate_us;
        assert!(
            phase_sum <= outer_us,
            "sub-phase sum {phase_sum} exceeds outer build wall {outer_us}"
        );
    }

    #[test]
    fn off_sample_frame_skips_decode_gate() {
        // W2: on a NON-sampled frame the decode-gate does NOT run (decode_gate_us
        // == 0) while the encode-side summary still produces the gate — and the
        // report is byte-identical to a forced-sample build (the sampled decode is
        // a pure cross-check, never a mutation of the output).
        let off = build_domain53_payload_sampled(
            proof_fixture_rows(),
            CURRENT_VALIDATED_SOURCE_PROOFS,
            0,
            false,
        );
        let on = build_domain53_payload_sampled(
            proof_fixture_rows(),
            CURRENT_VALIDATED_SOURCE_PROOFS,
            0,
            true,
        );
        assert_eq!(off.payload_phase_times.decode_gate_us, 0);
        // Sampling changes ONLY the timing, never payload/gate/errors.
        assert_eq!(off, on);
        assert_eq!(off.gate_summary, on.gate_summary);
        assert_eq!(off.errors, on.errors);
    }

    #[test]
    fn collector_emit_decision_matches_decode_derived_report() {
        // The collector gates emit on report.ok(), gate_summary.status_flags_or
        // (payload-failure + DELTA bits), and reports data_rows_total /
        // data_changed_rows_total. Prove those exact inputs are IDENTICAL whether
        // the gate summary is produced encode-side (production, W2) or by the old
        // per-frame decode — over the clean fixture AND a fail-closed (bad) one.
        for rows in [proof_fixture_rows(), {
            let mut r = proof_fixture_rows();
            r.opinions[0].opinion = MAX_OPINION_ABS + 1000; // force a parse error
            r
        }] {
            // New path (encode-side summary) via the public builder.
            let report = build_domain53_payload(rows.clone(), CURRENT_VALIDATED_SOURCE_PROOFS);
            // Reference: the pre-W2 behaviour = decode_payload on the same bytes.
            let decode_summary = decode_payload(0, &report.payload);

            // The four collector-visible fields must match the decode-derived ones.
            assert_eq!(
                report.gate_summary.status_flags_or, decode_summary.status_flags_or,
                "status_flags_or (emit gate) must match decode"
            );
            assert_eq!(
                report.gate_summary.data_rows_total, decode_summary.data_rows_total,
                "emitted_rows source must match decode"
            );
            assert_eq!(
                report.gate_summary.data_changed_rows_total, decode_summary.data_changed_rows_total,
                "changed_rows source must match decode"
            );
            // report.ok() is errors.is_empty(); the errors are the gate_summary
            // parse_errors, which must equal the decode parse_errors.
            assert_eq!(
                report.gate_summary.parse_errors, decode_summary.parse_errors,
                "parse_errors (which drive report.ok()) must match decode"
            );
            assert_eq!(
                report.ok(),
                decode_summary.parse_errors.is_empty(),
                "report.ok() must equal the decode-derived ok decision"
            );
        }
    }

    #[test]
    fn sampled_decode_mismatch_is_fail_closed() {
        // W2 safety net: when the encode-side summary and the sampled decode
        // DISAGREE (an encoder regression / heap tear), the build must fail closed.
        // Prove the wiring: `decode_verify_mismatch_error` returns Some on any
        // difference (which is pushed into `errors` → `report.ok()` false → the
        // collector nukes to SOURCE_UNAVAILABLE), and None when they agree.
        let a = build_domain53_payload(proof_fixture_rows(), CURRENT_VALIDATED_SOURCE_PROOFS)
            .gate_summary;
        assert!(
            decode_verify_mismatch_error(&a, &a).is_none(),
            "identical summaries must NOT fail closed"
        );

        let mut b = a.clone();
        b.data_rows_total += 1; // simulate a desync between bytes and snapshot
        let err = decode_verify_mismatch_error(&a, &b)
            .expect("differing summaries must produce a fail-closed error");
        assert!(err.contains("disagrees with sampled decode"));
        assert!(err.contains("fail-closed"));

        // End-to-end: a report whose errors contain such a mismatch is NOT ok, so
        // the collector's `!report.ok()` gate fires exactly as for a decode failure.
        let mut report =
            build_domain53_payload(proof_fixture_rows(), CURRENT_VALIDATED_SOURCE_PROOFS);
        assert!(
            report.ok(),
            "baseline fixture is clean: {:?}",
            report.errors
        );
        report.errors.push(err);
        assert!(
            !report.ok(),
            "a pushed mismatch error must make the report not-ok (fail closed)"
        );
    }

    #[test]
    fn payload_phase_times_are_ignored_in_report_equality() {
        // W1.2: `payload_phase_times` is observational wall-clock jitter. Two
        // builds of the SAME rows produce identical payload/gate/errors and MUST
        // compare equal even though their timing very likely differs.
        let a = build_domain53_payload(proof_fixture_rows(), CURRENT_VALIDATED_SOURCE_PROOFS);
        let b = build_domain53_payload(proof_fixture_rows(), CURRENT_VALIDATED_SOURCE_PROOFS);
        assert_eq!(a, b, "identical inputs must produce equal reports");
        // And the payload bytes / gate / errors are the semantic identity.
        assert_eq!(a.payload, b.payload);
        assert_eq!(a.gate_summary, b.gate_summary);
        assert_eq!(a.errors, b.errors);
    }

    #[test]
    fn rejects_country_war_projection_without_current_war_root() {
        let mut rows = proof_fixture_rows();
        rows.countries[0].at_war_count = 1;

        let report = build_domain53_payload(
            rows,
            CURRENT_VALIDATED_SOURCE_PROOFS & !SOURCE_PROOF_WAR_ROOT_870EBF,
        );

        assert!(!report.ok());
        assert!(report
            .errors
            .iter()
            .any(|error| error.contains("country at-war/casualty projections")));
    }

    #[test]
    fn rejects_war_rows_without_current_war_root() {
        let mut rows = proof_fixture_rows();
        rows.wars.push(WarRelationRow {
            first_tag_db_id: 1,
            second_tag_db_id: 3,
            first_casualties: 100,
            second_casualties: 200,
        });

        let report = build_domain53_payload(
            rows,
            CURRENT_VALIDATED_SOURCE_PROOFS & !SOURCE_PROOF_WAR_ROOT_870EBF,
        );

        assert!(!report.ok());
        assert!(report
            .errors
            .iter()
            .any(|error| error.contains("war rows require SOURCE_PROOF_WAR_ROOT_870EBF")));
    }

    #[test]
    fn rejects_incoming_action_rows_without_queue_source_proof() {
        let rows = proof_fixture_rows();

        let report = build_domain53_payload(
            rows,
            CURRENT_VALIDATED_SOURCE_PROOFS & !SOURCE_PROOF_INCOMING_ACTION_QUEUE_870EBF,
        );

        assert!(!report.ok());
        assert!(report
            .errors
            .iter()
            .any(|error| error.contains("incoming diplomatic action rows require")));
        assert_ne!(
            report.gate_summary.status_flags_or & STATUS_INCOMING_ACTION_QUEUE_UNVALIDATED,
            0
        );
    }

    #[test]
    fn rejects_intel_network_graph_rows_without_graph_source_proof() {
        let rows = proof_fixture_rows();

        let report = build_domain53_payload(
            rows,
            CURRENT_VALIDATED_SOURCE_PROOFS & !SOURCE_PROOF_INTEL_NETWORK_GRAPH_870EBF,
        );

        assert!(!report.ok());
        assert!(report
            .errors
            .iter()
            .any(|error| error.contains("intel-network graph rows require")));
        assert_ne!(
            report.gate_summary.status_flags_or & STATUS_INTEL_NETWORK_GRAPH_UNVALIDATED,
            0
        );
    }

    #[test]
    fn badges_war_lifecycle_detail_until_source_proof_exists() {
        let mut rows = proof_fixture_rows();
        rows.wars.push(WarRelationRow {
            first_tag_db_id: 1,
            second_tag_db_id: 3,
            first_casualties: 100,
            second_casualties: 200,
        });

        let report = build_domain53_payload(rows, CURRENT_VALIDATED_SOURCE_PROOFS);

        assert!(report.ok(), "{:?}", report.errors);
        assert_ne!(
            report.gate_summary.status_flags_or & STATUS_WAR_LIFECYCLE_DETAIL_UNVALIDATED,
            0
        );
        assert_eq!(
            report.gate_summary.status_flags_or & STATUS_WAR_ROOT_UNVALIDATED,
            0
        );
    }

    #[test]
    fn rejects_unresisted_operation_without_operation_detail_root() {
        let mut rows = proof_fixture_rows();
        rows.operation_anchors.push(OperationAnchorRow {
            owner_tag_db_id: 1,
            operation_id: 231_592,
            target_tag_db_id: Some(3),
            slot_count: 2,
            phase_count: 3,
            detail_resisted: false,
        });

        let report = build_domain53_payload(rows, CURRENT_VALIDATED_SOURCE_PROOFS);

        assert!(!report.ok());
        assert!(report
            .errors
            .iter()
            .any(|error| error.contains("unresisted operation rows require")));
    }

    #[test]
    fn rejects_unresisted_intel_map_without_percentage_root() {
        let mut rows = proof_fixture_rows();
        rows.intel_map.push(IntelMapRow {
            owner_tag_db_id: 1,
            target_tag_db_id: 3,
            civilian_raw: 10_000,
            army_raw: 0,
            navy_raw: 0,
            air_raw: 0,
            resisted: false,
        });

        let report = build_domain53_payload(rows, CURRENT_VALIDATED_SOURCE_PROOFS);

        assert!(!report.ok());
        assert!(report
            .errors
            .iter()
            .any(|error| error.contains("unresisted intel-map rows require")));
    }

    #[test]
    fn rejects_state_events_without_matching_source_proof() {
        let mut rows = proof_fixture_rows();
        rows.operatives.clear();
        rows.state_events
            .retain(|row| row.source_kind == STATE_EVENT_SOURCE_OPERATIVE_LIFECYCLE);

        let report = build_domain53_payload(
            rows,
            CURRENT_VALIDATED_SOURCE_PROOFS & !SOURCE_PROOF_OPERATIVE_ROOT_870EBF,
        );

        assert!(!report.ok());
        assert!(report
            .errors
            .iter()
            .any(|error| error.contains("compact state events sourced from operative lifecycle")));
    }

    #[test]
    fn rejects_faction_group_state_event_without_country_source_proof() {
        let mut rows = proof_fixture_rows();
        rows.countries.clear();
        rows.state_events
            .retain(|row| row.source_kind == STATE_EVENT_SOURCE_COUNTRY_FACTION_GROUP);

        let report = build_domain53_payload(
            rows,
            CURRENT_VALIDATED_SOURCE_PROOFS & !SOURCE_PROOF_COUNTRY_ROOTS_870EBF,
        );

        assert!(!report.ok());
        assert!(report
            .errors
            .iter()
            .any(|error| error.contains("compact state events sourced from country faction")));
    }

    #[test]
    fn root_manifest_pins_current_870ebf_subset() {
        let evidence = CURRENT_BUILD_ROOTS_870EBF
            .iter()
            .map(|root| root.evidence)
            .collect::<Vec<_>>()
            .join("\n");

        assert!(evidence.contains("module+0x3316E90"));
        assert!(evidence.contains("module+0x296B338"));
        assert!(evidence.contains("module+0x29A6540"));
        assert!(evidence.contains("module+0x293F790"));
        assert!(evidence.contains("module+0x2702388"));
        assert!(evidence.contains("module+0x298B448"));
        assert!(evidence.contains("module+0x298AB48"));
        assert_ne!(
            CURRENT_VALIDATED_SOURCE_PROOFS & SOURCE_PROOF_WAR_ROOT_870EBF,
            0
        );
        assert_ne!(
            CURRENT_VALIDATED_SOURCE_PROOFS & SOURCE_PROOF_INTEL_NETWORK_GRAPH_870EBF,
            0
        );
    }

    #[test]
    fn intentional_omissions_do_not_set_partial_or_source_failure_bits() {
        let report = build_domain53_payload(proof_fixture_rows(), CURRENT_VALIDATED_SOURCE_PROOFS);

        assert!(report.ok(), "{:?}", report.errors);
        assert_eq!(
            report.gate_summary.status_flags_or
                & (crate::compact::STATUS_SOURCE_UNAVAILABLE | crate::compact::STATUS_PARTIAL_DATA),
            0
        );
        assert_ne!(
            report.gate_summary.status_flags_or
                & (STATUS_OPERATIONS_DETAIL_RESISTED
                    | STATUS_INTEL_MAP_RESISTED
                    | STATUS_WAR_LIFECYCLE_DETAIL_UNVALIDATED),
            0
        );
        assert_eq!(
            report.gate_summary.status_flags_or & STATUS_INCOMING_ACTION_QUEUE_UNVALIDATED,
            0
        );
        assert_eq!(
            report.gate_summary.status_flags_or & STATUS_INTEL_NETWORK_GRAPH_UNVALIDATED,
            0
        );
        assert_eq!(
            report.gate_summary.status_flags_or & STATUS_WAR_ROOT_UNVALIDATED,
            0
        );
    }

    #[test]
    fn unresolved_operation_and_intel_gaps_are_flagged_without_rows() {
        let report = build_domain53_payload(proof_fixture_rows(), CURRENT_VALIDATED_SOURCE_PROOFS);

        assert!(report.ok(), "{:?}", report.errors);
        assert!(report.snapshot.operation_anchors.is_empty());
        assert!(report.snapshot.intel_map.is_empty());
        assert_ne!(
            report.gate_summary.status_flags_or & STATUS_OPERATIONS_DETAIL_RESISTED,
            0
        );
        assert_ne!(
            report.gate_summary.status_flags_or & STATUS_INTEL_MAP_RESISTED,
            0
        );
        assert_eq!(
            report
                .gate_summary
                .data_units_total
                .get("operation_anchor_rows")
                .copied(),
            Some(0)
        );
        assert_eq!(
            report
                .gate_summary
                .data_units_total
                .get("intel_map_rows")
                .copied(),
            Some(0)
        );
    }

    // P4c-2: prove the batched relation-table read (`relation_slot_ptr` sourcing
    // from the whole-array `read_bytes` slice) is byte-identical to the original
    // per-slot `read_u64(relation_table + tag*8)` for EVERY slot, and that the
    // absent-batch path falls back to the exact per-slot read (torn slot => the
    // same `Err`). This is the P4c-2 pass/fail criterion (byte-identical output)
    // for Target 1.
    #[test]
    fn relation_slot_batch_matches_per_field_read() {
        let module = 0x1_4000_0000u64;
        let heap = profile::DOMAIN53_HEAP_LO as u64 + 0x10_0000;
        let relation_table = heap + 0x1_0000;

        // A relation table whose slots carry a representative mix: 0 (empty),
        // a heap pointer, a non-heap sentinel, and a large value -- the exact
        // kinds the caller's match arms distinguish (heap / 0 / other).
        let max_tag: u32 = 45; // matches FAKE_COUNTRY_ROWS scale
        let slots = [
            (1u32, 0u64),         // empty slot
            (2, heap + 0x9_9000), // heap pointer
            (3, 0xDEAD_BEEFu64),  // non-heap, non-zero
            (7, heap + 0x1234),   // heap pointer
            (max_tag, u64::MAX),  // max index, extreme value
        ];

        let mut mem = FakeMemory::new(module);
        // Table chunk sized exactly like the reader batches: (max_tag+1)*8.
        mem.add_chunk(relation_table, (max_tag as usize + 1) * 8);
        for (tag, value) in slots {
            mem.write_u64(relation_table + u64::from(tag) * 8, value);
        }

        // Batch the whole array once, exactly as read_relation_rows does.
        let batch_len = (max_tag as usize + 1) * 8;
        let table_bytes = mem.read_bytes(relation_table, batch_len).ok();
        assert!(table_bytes.is_some(), "batch read of a committed table");

        for (tag, expected) in slots {
            // Per-field reference: the original inline `read_u64(entry_slot)`.
            let entry_slot = relation_table + u64::from(tag) * 8;
            let per_field = mem.read_u64(entry_slot);

            // Batched path.
            let batched =
                relation_slot_ptr(&mem, table_bytes.as_deref(), relation_table, tag).unwrap();

            assert_eq!(
                batched, per_field,
                "batched slot {tag} diverged from per-field read"
            );
            assert_eq!(batched, Ok(expected), "slot {tag} value");
        }

        // Absent-batch fallback: identical result, sourced from read_u64.
        for (tag, _expected) in slots {
            let entry_slot = relation_table + u64::from(tag) * 8;
            let per_field = mem.read_u64(entry_slot);
            let fallback = relation_slot_ptr(&mem, None, relation_table, tag).unwrap();
            assert_eq!(fallback, per_field, "fallback slot {tag}");
        }

        // Torn slot with NO batch: the per-slot read fails, and the fallback must
        // surface the SAME `Err` (caller turns this into `acc.skip()`), never a
        // fabricated pointer.
        let torn_table = heap + 0x5_0000; // no chunk registered -> unreadable
        let torn = relation_slot_ptr(&mem, None, torn_table, 2).unwrap();
        assert!(
            torn.is_err(),
            "unreadable slot must fail-closed (Err) in the fallback path"
        );
        assert!(mem.read_u64(torn_table + 16).is_err());
    }

    // (The inline-scan qword range-check helpers + their equivalence test were removed
    // with the inline scan itself: relation-flag sub-objects live in the `rel+0xD0`
    // container, not inline, so there is no per-qword blob scan to fast-path any more.
    // See `read_relation_container` and its tests below.)

    /// The repurposed relation-flag diagnostic counts container sub-objects PER KIND:
    /// `per_kind[K].0` is a plain COUNT (`.1` stays 0) and `total_matches` is the sum.
    /// A default histogram is empty. (Post inline-scan removal — see the struct doc.)
    #[test]
    fn relflag_histogram_counts_per_kind() {
        let mut hist = RelationFlagOffsetHistogram::default();
        assert_eq!(hist.total_matches, 0);
        assert!(hist.per_kind.is_empty());
        hist.record_kind_count(REL_KIND_NON_AGGRESSION_PACT);
        hist.record_kind_count(REL_KIND_NON_AGGRESSION_PACT);
        hist.record_kind_count(REL_KIND_NON_AGGRESSION_PACT);
        hist.record_kind_count(REL_KIND_EMBARGO);
        hist.record_kind_count(REL_KIND_PUPPET);
        hist.record_kind_count(REL_KIND_PUPPET);
        assert_eq!(hist.per_kind[&REL_KIND_NON_AGGRESSION_PACT], (3, 0));
        assert_eq!(hist.per_kind[&REL_KIND_EMBARGO], (1, 0));
        assert_eq!(hist.per_kind[&REL_KIND_PUPPET], (2, 0));
        assert_eq!(hist.total_matches, 6);
    }

    /// Build a `rel` object for the pair (1, 2) with a `pdx::vector<CRelation*>` (`+0xD0`
    /// begin / `+0xDC` count) holding a mix of sub-objects, and assert
    /// `read_relation_container`:
    ///   - emits owned kinds once (NAP 1->2 with a real end_date; military_access stored
    ///     REVERSE as 2->1 with the sentinel end -> emitted 2->1, no end event),
    ///   - COUNTS but does NOT emit a non-d53 kind (air_base_access -> domain 30),
    ///   - skips an unknown-vtable sibling and a null element QUIETLY (no counted skip),
    ///   - dedups a duplicate NAP entry (same object appears in both direction buckets).
    #[test]
    fn read_relation_container_emits_owned_kinds_and_dedups() {
        let module = 0x1_4000_0000u64;
        let heap = profile::DOMAIN53_HEAP_LO as u64 + 0x10_0000;
        let vtables = relation_vtable_kinds(module).expect("vtables");
        let rel = heap + 0x1000;
        let arr = heap + 0x2000;
        let nap = heap + 0x3000;
        let milacc = heap + 0x3100;
        let airbase = heap + 0x3200;
        let unknown = heap + 0x3300;

        let mut mem = FakeMemory::new(module);
        for (addr, len) in [
            (rel, 0x100usize),
            (arr, 0x40),
            (nap, 0x40),
            (milacc, 0x40),
            (airbase, 0x40),
            (unknown, 0x40),
        ] {
            mem.add_chunk(addr, len);
        }
        // Vector header: begin=arr, cap=8, count=6.
        mem.write_u64(rel + profile::DOMAIN53_RELATION_SUBREL_BEGIN as u64, arr);
        mem.write_i32(rel + profile::DOMAIN53_RELATION_SUBREL_CAP as u64, 8);
        mem.write_i32(rel + profile::DOMAIN53_RELATION_SUBREL_COUNT as u64, 6);
        // Elements: [nap, milacc, airbase, unknown, null, nap(dup)]. All belong to pair (1,2).
        mem.write_u64(arr, nap);
        mem.write_u64(arr + 8, milacc);
        mem.write_u64(arr + 16, airbase);
        mem.write_u64(arr + 24, unknown);
        mem.write_u64(arr + 32, 0);
        mem.write_u64(arr + 40, nap);
        // NAP (1->2), start 100, real end 200.
        mem.write_u64(
            nap,
            module + profile::DOMAIN53_REL_NON_AGGRESSION_PACT_VTABLE_RVA as u64,
        );
        mem.write_u32(nap + profile::DOMAIN53_REL_BASE_FIRST as u64, 1);
        mem.write_u32(nap + profile::DOMAIN53_REL_BASE_SECOND as u64, 2);
        mem.write_i32(nap + profile::DOMAIN53_REL_BASE_START_DATE as u64, 100);
        mem.write_i32(nap + profile::DOMAIN53_REL_BASE_END_DATE as u64, 200);
        // MILITARY_ACCESS stored REVERSE (2->1) — same bucket {1,2}, so it is valid and
        // emitted with its own direction (2->1). start 50, end = sentinel (no end).
        mem.write_u64(
            milacc,
            module + profile::DOMAIN53_REL_MILITARY_ACCESS_VTABLE_RVA as u64,
        );
        mem.write_u32(milacc + profile::DOMAIN53_REL_BASE_FIRST as u64, 2);
        mem.write_u32(milacc + profile::DOMAIN53_REL_BASE_SECOND as u64, 1);
        mem.write_i32(milacc + profile::DOMAIN53_REL_BASE_START_DATE as u64, 50);
        mem.write_i32(
            milacc + profile::DOMAIN53_REL_BASE_END_DATE as u64,
            profile::DOMAIN53_REL_DATE_SENTINEL,
        );
        // AIR_BASE_ACCESS (1->2): counted in the diagnostic, NOT emitted (domain 30 owns it).
        mem.write_u64(
            airbase,
            module + profile::DOMAIN53_REL_AIR_BASE_ACCESS_VTABLE_RVA as u64,
        );
        mem.write_u32(airbase + profile::DOMAIN53_REL_BASE_FIRST as u64, 1);
        mem.write_u32(airbase + profile::DOMAIN53_REL_BASE_SECOND as u64, 2);
        mem.write_i32(airbase + profile::DOMAIN53_REL_BASE_START_DATE as u64, 60);
        mem.write_i32(
            airbase + profile::DOMAIN53_REL_BASE_END_DATE as u64,
            profile::DOMAIN53_REL_DATE_SENTINEL,
        );
        // Unknown vtable (not in the relation map) -> skipped quietly, not counted.
        mem.write_u64(unknown, module + 0x0099_9999);

        let mut acc = RowAccumulator::default();
        let mut seen_flags = BTreeSet::new();
        read_relation_container(&mem, rel, &vtables, 1, 2, 1000, &mut acc, &mut seen_flags);

        // Rows: NAP(1,2), MILITARY_ACCESS(2,1) in element order. air_base absent.
        assert_eq!(
            acc.rows.relation_flags,
            vec![
                RelationFlagRow {
                    src_tag_db_id: 1,
                    target_tag_db_id: 2,
                    flag_kind: REL_KIND_NON_AGGRESSION_PACT,
                    directed: true,
                },
                RelationFlagRow {
                    src_tag_db_id: 2,
                    target_tag_db_id: 1,
                    flag_kind: REL_KIND_MILITARY_ACCESS,
                    directed: true,
                },
            ],
        );
        // Diagnostic counts EVERY known-kind entry BEFORE the emit filter + dedup: NAP
        // twice (incl. the duplicate), military_access once, air_base_access once.
        assert_eq!(
            acc.relflag_offsets.per_kind[&REL_KIND_NON_AGGRESSION_PACT],
            (2, 0)
        );
        assert_eq!(
            acc.relflag_offsets.per_kind[&REL_KIND_MILITARY_ACCESS],
            (1, 0)
        );
        assert_eq!(
            acc.relflag_offsets.per_kind[&REL_KIND_AIR_BASE_ACCESS],
            (1, 0)
        );
        assert_eq!(acc.relflag_offsets.total_matches, 4);
        // No torn reads -> no counted skips (null + unknown are QUIET skips).
        assert_eq!(acc.skipped_rows, 0);
        assert!(seen_flags.contains(&(1, 2, REL_KIND_NON_AGGRESSION_PACT)));
        assert!(seen_flags.contains(&(2, 1, REL_KIND_MILITARY_ACCESS)));
        // Flag state_events: NAP -> kind+start+end (3), military_access -> kind+start (2).
        let flag_events = acc
            .rows
            .state_events
            .iter()
            .filter(|e| e.source_kind == STATE_EVENT_SOURCE_RELATION_FLAG)
            .count();
        assert_eq!(flag_events, 5);
        // The NAP end_date event carries raw 200; the sentinel end is never emitted.
        assert!(acc.rows.state_events.iter().any(|e| e.value_raw == 200));
        assert!(!acc
            .rows
            .state_events
            .iter()
            .any(|e| e.value_raw == i64::from(profile::DOMAIN53_REL_DATE_SENTINEL)));
    }

    /// Fail-closed: an empty container and a null begin yield nothing WITHOUT a counted
    /// skip; a garbage count header and a torn element pointer each count exactly one
    /// skip and fabricate no row.
    #[test]
    fn read_relation_container_fail_closed() {
        let module = 0x1_4000_0000u64;
        let heap = profile::DOMAIN53_HEAP_LO as u64 + 0x10_0000;
        let vtables = relation_vtable_kinds(module).expect("vtables");
        let rel = heap + 0x1000;
        let arr = heap + 0x2000;

        // (a) Empty container (begin heap, count 0): nothing, no skip.
        {
            let mut mem = FakeMemory::new(module);
            mem.add_chunk(rel, 0x100);
            mem.add_chunk(arr, 0x40);
            mem.write_u64(rel + profile::DOMAIN53_RELATION_SUBREL_BEGIN as u64, arr);
            mem.write_i32(rel + profile::DOMAIN53_RELATION_SUBREL_CAP as u64, 4);
            mem.write_i32(rel + profile::DOMAIN53_RELATION_SUBREL_COUNT as u64, 0);
            let mut acc = RowAccumulator::default();
            let mut seen = BTreeSet::new();
            read_relation_container(&mem, rel, &vtables, 1, 2, 1000, &mut acc, &mut seen);
            assert!(acc.rows.relation_flags.is_empty());
            assert_eq!(acc.skipped_rows, 0);
        }
        // (b) Null begin: no sub-relations, no skip (the common case).
        {
            let mut mem = FakeMemory::new(module);
            mem.add_chunk(rel, 0x100);
            mem.write_u64(rel + profile::DOMAIN53_RELATION_SUBREL_BEGIN as u64, 0);
            let mut acc = RowAccumulator::default();
            let mut seen = BTreeSet::new();
            read_relation_container(&mem, rel, &vtables, 1, 2, 1000, &mut acc, &mut seen);
            assert!(acc.rows.relation_flags.is_empty());
            assert_eq!(acc.skipped_rows, 0);
        }
        // (c) Garbage count (> capacity): one counted skip, no row.
        {
            let mut mem = FakeMemory::new(module);
            mem.add_chunk(rel, 0x100);
            mem.add_chunk(arr, 0x40);
            mem.write_u64(rel + profile::DOMAIN53_RELATION_SUBREL_BEGIN as u64, arr);
            mem.write_i32(rel + profile::DOMAIN53_RELATION_SUBREL_CAP as u64, 2);
            mem.write_i32(rel + profile::DOMAIN53_RELATION_SUBREL_COUNT as u64, 9999);
            let mut acc = RowAccumulator::default();
            let mut seen = BTreeSet::new();
            read_relation_container(&mem, rel, &vtables, 1, 2, 1000, &mut acc, &mut seen);
            assert!(acc.rows.relation_flags.is_empty());
            assert_eq!(acc.skipped_rows, 1);
        }
        // (d) Torn element: the slot holds a heap-range pointer into an UNREGISTERED
        // chunk, so the vptr read fails -> one counted skip, no row.
        {
            let mut mem = FakeMemory::new(module);
            mem.add_chunk(rel, 0x100);
            mem.add_chunk(arr, 0x40);
            mem.write_u64(rel + profile::DOMAIN53_RELATION_SUBREL_BEGIN as u64, arr);
            mem.write_i32(rel + profile::DOMAIN53_RELATION_SUBREL_CAP as u64, 4);
            mem.write_i32(rel + profile::DOMAIN53_RELATION_SUBREL_COUNT as u64, 1);
            mem.write_u64(arr, heap + 0x9_0000);
            let mut acc = RowAccumulator::default();
            let mut seen = BTreeSet::new();
            read_relation_container(&mem, rel, &vtables, 1, 2, 1000, &mut acc, &mut seen);
            assert!(acc.rows.relation_flags.is_empty());
            assert_eq!(acc.skipped_rows, 1);
        }
        // (e) Wrong-pair entry: a valid NAP object for pair (3,4) sitting in the (1,2)
        // bucket is REJECTED (counted skip), never emitted as a fabricated wrong-pair row.
        {
            let obj = heap + 0x4000;
            let mut mem = FakeMemory::new(module);
            mem.add_chunk(rel, 0x100);
            mem.add_chunk(arr, 0x40);
            mem.add_chunk(obj, 0x40);
            mem.write_u64(rel + profile::DOMAIN53_RELATION_SUBREL_BEGIN as u64, arr);
            mem.write_i32(rel + profile::DOMAIN53_RELATION_SUBREL_CAP as u64, 4);
            mem.write_i32(rel + profile::DOMAIN53_RELATION_SUBREL_COUNT as u64, 1);
            mem.write_u64(arr, obj);
            mem.write_u64(
                obj,
                module + profile::DOMAIN53_REL_NON_AGGRESSION_PACT_VTABLE_RVA as u64,
            );
            mem.write_u32(obj + profile::DOMAIN53_REL_BASE_FIRST as u64, 3);
            mem.write_u32(obj + profile::DOMAIN53_REL_BASE_SECOND as u64, 4);
            mem.write_i32(obj + profile::DOMAIN53_REL_BASE_START_DATE as u64, 100);
            mem.write_i32(obj + profile::DOMAIN53_REL_BASE_END_DATE as u64, 200);
            let mut acc = RowAccumulator::default();
            let mut seen = BTreeSet::new();
            // Bucket is (1,2) but the object is (3,4) -> rejected.
            read_relation_container(&mem, rel, &vtables, 1, 2, 1000, &mut acc, &mut seen);
            assert!(acc.rows.relation_flags.is_empty());
            assert_eq!(acc.skipped_rows, 1);
        }
    }

    // T2 deliverable (d): the precomputed opinion-label cache indexed by
    // `(opinion + MAX_OPINION_ABS)` must be byte-for-byte identical to the
    // original `format!("opinion_{opinion}")` for every opinion in the validated
    // domain — hashes included (the state_event only stores `stable_hash32`).
    #[test]
    fn opinion_labels_match_format_over_full_domain() {
        let opinion_labels: Vec<String> = (-MAX_OPINION_ABS..=MAX_OPINION_ABS)
            .map(|opinion| format!("opinion_{opinion}"))
            .collect();
        assert_eq!(opinion_labels.len(), (2 * MAX_OPINION_ABS + 1) as usize);

        // Full domain byte-identity + hash-identity.
        for opinion in -MAX_OPINION_ABS..=MAX_OPINION_ABS {
            let idx = (opinion + MAX_OPINION_ABS) as usize;
            let expected = format!("opinion_{opinion}");
            assert_eq!(opinion_labels[idx], expected, "label bytes for {opinion}");
            assert_eq!(
                crate::compact::stable_hash32(&opinion_labels[idx]),
                crate::compact::stable_hash32(&expected),
                "label hash for {opinion}"
            );
        }

        // Explicit boundary values called out by the spec.
        for opinion in [-MAX_OPINION_ABS, -1, 0, 1, MAX_OPINION_ABS] {
            let idx = (opinion + MAX_OPINION_ABS) as usize;
            assert_eq!(opinion_labels[idx], format!("opinion_{opinion}"));
        }
        // Spot-check the exact spelling of the extremes.
        assert_eq!(opinion_labels[0], "opinion_-200");
        assert_eq!(opinion_labels[MAX_OPINION_ABS as usize], "opinion_0");
        assert_eq!(
            opinion_labels[(2 * MAX_OPINION_ABS) as usize],
            "opinion_200"
        );
    }
}
