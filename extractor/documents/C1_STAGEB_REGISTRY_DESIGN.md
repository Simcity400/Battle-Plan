# C1 stage B: closed declaration registry and transactional epoch tokens

Status: decision-complete design for implementation. This document designs C1 stage B only. It
adds no declaration kind and tolerates no `(domain, segment, cause)` pair.

## 1. Binding inputs and scope

The binding requirements are the G4 amendments in
`extractor/documents/CONFIRMED_COMPLETE_DESIGN.md:222-300`. In particular:

- the declaration-kind registry is closed and deny-by-default, and its initial tolerance set is
  empty (`extractor/documents/CONFIRMED_COMPLETE_DESIGN.md:227-234`);
- declarations and frames carry the same six-part epoch token, with sealing at the first sign
  that reload may have begun (`extractor/documents/CONFIRMED_COMPLETE_DESIGN.md:236-242`);
- every active declaration is checked every frame either by re-reading its immutable token or by
  an independent bounded-skew presence sentinel
  (`extractor/documents/CONFIRMED_COMPLETE_DESIGN.md:244-248`);
- raw domain status ORs never grant tolerance; only per-attempt, per-subsource evidence can be
  accounted (`extractor/documents/CONFIRMED_COMPLETE_DESIGN.md:250-256`); and
- constructors are private, the wire change requires a new major, and the required adversarial
  sequences and mutations are gates
  (`extractor/documents/CONFIRMED_COMPLETE_DESIGN.md:265-278`).

This design consumes, but does not invent, campaign identity. The product contract says that an
engine-derived cross-session campaign identity is still unbuilt and must eventually be stamped
into every stream (`extractor/documents/PRODUCT_SESSION_SEMANTICS.md:34-39`). Therefore the
campaign-identity track owns campaign derivation and cross-session stitching. Stage B owns only
the in-session `save_epoch_generation` boundary and exposes the interface in section 4.5. It must
not derive campaign identity from wall clock, process identity, save path, or a negative memory
search. Any future session-lifecycle specification must continue to cite that product contract
(`extractor/documents/PRODUCT_SESSION_SEMANTICS.md:68-69`).

The existing `COMPACT_EXPECTED_PARTIAL_TOLERANCES` table is unrelated. It describes reviewed
PARTIAL fold behavior (`extractor/native/compact_contract/src/lib.rs:228-323`); it cannot create
an ExpectedAbsent disposition and it is not the registry designed here.

### 1.1 Deliverable boundary

Stage B defines the contract types, the closed registry, epoch state machine, mismatch sentinel,
and the Stage C consumption interface. Stage B does not:

- add an allowed declaration kind;
- change any recorder gate or fold behavior;
- classify any C0 incidence as ExpectedAbsent;
- implement campaign identity or rollback stitching; or
- enable the new wire major (that is Stage E).

The no-argument C0 census currently obtains its executable domain roster from the compact
contract and reports the remaining blocking incidences. Its own source describes the census as
manifest-level discovery, not per-hour classification
(`extractor/native/scripts/c0_domain_census.py:1-16`). Nothing in this document converts a C0
incidence into a declaration kind.

## 2. Non-negotiable invariants

1. `ExpectedAbsentDeclaration` and `Disposition::ExpectedAbsent` have no public constructor and
   no public fields. Only the registry module can construct them.
2. `DECLARATION_KINDS` is an empty array in the Stage B implementation. Every scheduled segment
   policy is explicitly `Forbidden`; there is no wildcard/default policy.
3. A declaration, its reference, its sentinel result, its failure-evidence units, and its frame
   compare equal to one full `EpochToken`. Hash equality is never a substitute for field equality.
4. Capture stops at `ReloadMayHaveBegun`, not at later confirmation. An uncertain or overlapping
   frame is `Failed`.
5. The old epoch is sealed and recorder-acknowledged over the H4CT duplex control channel before
   the collector may publish a new declaration table or frame. An H4CR write, transport send
   success, timeout, disconnect, or stream-history inference is never an acknowledgement.
6. Aggregate status bits and `so_*` manifest aggregates cannot excuse a failure. They can only
   prove that an exact failure cause was observed.
7. A presence hit wins over a declaration: revoke first, run the ordinary reader, mark the frame
   Failed if that reader is incomplete, and fail the run in either case.
8. No production input path accepts declaration kinds, declarations, policy overrides, sentinel
   cadence, evidence-reader ids, proof ids, or absence lists from users or files.
9. Unknown ids, enum discriminants, schema versions, token fields, declaration references, and
   roster entries reject. There is no `Unknown`, `Other`, or best-effort variant.
10. Any arithmetic overflow in nonce counters, attempt ids, generations, ordinals, or accounting
    counters is terminal and fails the run.

## 3. Module and ownership shape

The implementation should put shared types and the closed tables in `compact_contract`, with
collector issuance/state in `collector` and recorder validation/accounting in `recorder_host`:

```text
compact_contract::dispositions
  ids and enums; EpochToken; declaration/frame/evidence wire schemas
compact_contract::scheduled_segments
  exhaustive segment roster derived alongside ASSIGNED_COMPACT_DOMAINS
compact_contract::declaration_registry
  private constructors; explicit policy table; EMPTY kind table
compact_contract::status_origin_descriptors
  closed mapping from Stage A so_* ids to segment/cause metadata
collector::epoch_transaction
  reload fence, issue/revoke/seal state machine, sentinel scheduling
recorder_host::disposition_accounting
  exact-token validation, per-segment partition, run gate
```

`ASSIGNED_COMPACT_DOMAINS` is already the executable domain registry
(`extractor/native/compact_contract/src/lib.rs:638-640`), and each descriptor already says whether
the domain is schedulable (`extractor/native/compact_contract/src/lib.rs:375-384`). The segment
roster is a new,
finer contract keyed to that table; it must not revive the retired hand-maintained domain table
(`extractor/documents/DOMAIN_REGISTRY.md:1-18`).

## 4. Exact Rust-level schema

The snippets below define field shape and visibility. Names may move between files, but an
implementation is conforming only if the serialized fields and invariants remain exact.

### 4.1 IDs and canonical encoding

```rust
pub type DomainId = u16;

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Hash)]
pub struct SegmentId(u16);

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Hash)]
pub struct SegmentKey {
    pub domain_id: DomainId,
    pub segment_id: SegmentId,
}

macro_rules! digest_id {
    ($name:ident) => {
        #[repr(transparent)]
        #[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Hash)]
        pub struct $name([u8; 32]);
    };
}

digest_id!(DeclarationKindId);
digest_id!(EvidenceFieldId);
digest_id!(EvidenceReaderId);
digest_id!(ImmutabilityProofId);
digest_id!(ProofTestId);
digest_id!(EvidenceDigest);
digest_id!(ModsetHash);
digest_id!(ExeHash);
digest_id!(CampaignIdentityDigest); // interface owned by the campaign track

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Hash)]
pub struct SchedulerEntryPointId(u16);
```

All digest ids are SHA-256 over a domain-separated canonical byte encoding. Canonical encoding is
fixed-width little-endian integers, length-prefixed UTF-8 only where a closed symbolic name must
be encoded, declaration fields in the order shown below, and no maps, JSON, platform paths,
addresses, or Rust debug strings. Each hash preimage begins with an ASCII purpose tag and its
schema version, for example `b"H4CR:C1:DECLARATION:v1\0"`. Digests are indices and corruption
checks, never equality shortcuts.

Canonical optional fixed-width fields use a one-byte presence tag, never a sentinel value:
`0 = None` and is followed by no value bytes; `1 = Some` and is followed immediately by the
canonical value bytes. Any other tag, a missing `Some` value, bytes inserted after `None`, or
trailing bytes rejects. The field following the option therefore begins immediately after either
the tag or the tagged value, and exact payload length is part of validation.

`SegmentId` values and symbolic names are assigned in the executable segment roster. Zero is
invalid. A `SegmentKey` is exact: a declaration for one subsource never applies to its sibling or
whole domain.

### 4.2 Epoch token

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SessionNonce([u8; 32]);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProcessStartIdentity {
    pub pid: u32,
    pub create_time_100ns: u64,
}

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct SaveEpochGeneration(u64);

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EpochSchemaVersion(u32);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EpochToken {
    pub session_nonce: SessionNonce,
    pub process_start: ProcessStartIdentity,
    pub save_epoch_generation: SaveEpochGeneration,
    pub modset_hash: ModsetHash,
    pub exe_hash: ExeHash,
    pub schema_version: EpochSchemaVersion,
}
```

The six fields are the complete G4 epoch token. No field is optional.

- `SessionNonce` is generated from the operating-system CSPRNG only inside section 5.1's bounded
  Opening transaction; all-zero is rejected. It is never read from configuration and never reused
  intentionally.
- `ProcessStartIdentity` requires both PID and the process creation FILETIME. PID alone is not
  accepted because it is reusable. Existing process-identity work identifies PID plus creation
  time as the stable process-instance key and calls module base informational only
  (`extractor/live_capture/process_identity.py:8-17`). Its current Python pid-only fallback
  (`extractor/live_capture/process_identity.py:133-142`) is explicitly not permitted for this
  contract. Process-handle acquisition and both identity queries are section-5.1 Opening worker
  operations; a cached PID or a query outside the Opening deadline cannot mint a token.
- `SaveEpochGeneration` starts at 1, increases exactly once for each opened save epoch within a
  session, and never wraps. A possible reload closes the current generation before the next one
  can be opened.
- `ModsetHash` is SHA-256 of the canonical, fully resolved modset binding, including enabled mod
  content hashes and the DLC census. An unresolved/partial modset cannot mint a token. The current
  resolver already treats uncertainty as unresolved rather than vanilla
  (`extractor/native/recorder_host/src/modset.rs:1-12`) and records content SHA-256
  (`extractor/native/recorder_host/src/modset.rs:285-301`). Enumeration, content reads, and hashing
  all run under the one unchanged section-5.1 Opening deadline.
- `ExeHash` is the stable SHA-256 obtained from the attached process image binding. Failure to
  query or hash it prevents token creation
  (`extractor/native/recorder_host/src/modset.rs:353-376`). Image query, file/content read, and
  hashing are all section-5.1 Opening worker operations under that same deadline.
- `EpochSchemaVersion` changes for any incompatible canonical token, declaration, disposition,
  or evidence schema change. It is not a domain payload version. Stage E maps this contract to a
  new top-level stream major; the current H4CR record version is 1
  (`extractor/native/compact_contract/src/lib.rs:17-25`).

An optional `EpochTokenDigest` may index maps, but validators first compare all six fields and only
then may verify/recompute the digest. No timestamp, module base, filesystem path, local machine
identifier, or memory address participates.

### 4.3 Closed scheduled-segment roster and policy table

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScheduledSegmentDescriptor {
    pub key: SegmentKey,
    pub canonical_name: &'static str,
    pub scheduled_by: SchedulerEntryPointId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeclarationPolicy {
    Forbidden,
    Allowed(DeclarationKindId),
}

pub const SCHEDULED_SEGMENTS: &[ScheduledSegmentDescriptor] = &[/* exhaustive */];
pub const DECLARATION_POLICIES: &[(SegmentKey, DeclarationPolicy)] = &[
    /* exactly one explicit entry per SCHEDULED_SEGMENTS key; all Forbidden in stage B */
];
```

There is deliberately no `policy_for(key).unwrap_or(Forbidden)` implementation. Registry
initialization requires set equality between schedulable segment keys and policy keys, plus
unique ids/names and a valid parent in `ASSIGNED_COMPACT_DOMAINS`. Missing, duplicate, stale, or
extra policy entries are contract errors. Consequently, adding a segment to the roster without
editing the policy table fails tests even though the intended eventual answer might be
`Forbidden`.

The Stage B table contains an explicit `Forbidden` for every roster key. It contains zero
`Allowed` values.

### 4.4 Declaration-kind registry and declaration record

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvalidationEvent {
    ReloadMayHaveBegun,
    ProcessIdentityChanged,
    SaveEpochChanged,
    ModsetChanged,
    ExeChanged,
    SchemaChanged,
    AuthoritativeEvidenceChanged,
    SweepGenerationDied,
    SweepGenerationSuperseded,
    PresenceObserved,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MismatchDetectorSpec {
    RevalidateAuthoritativeTokenEveryFrame,
    PositivePresenceSentinel {
        sentinel_reader: EvidenceReaderId,
        every_attempts: NonZeroU32, // registry validation requires exactly 1
        max_skew_attempts: NonZeroU32, // registry validation requires exactly 1
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeclarationKindSpec {
    pub kind_id: DeclarationKindId,
    pub segment: SegmentKey,
    pub authoritative_evidence_field: EvidenceFieldId,
    pub authoritative_evidence_reader: EvidenceReaderId,
    pub immutability_proof_id: ImmutabilityProofId,
    pub invalidation_event: InvalidationEvent,
    pub proof_test_id: ProofTestId,
    pub mismatch_detector: MismatchDetectorSpec,
}

pub const DECLARATION_KINDS: [DeclarationKindSpec; 0] = [];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeclarationId {
    pub epoch_digest: [u8; 32],
    pub content_sha256: [u8; 32],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExpectedAbsentDeclaration {
    id: DeclarationId,
    epoch: EpochToken,
    kind_id: DeclarationKindId,
    segment: SegmentKey,
    authoritative_evidence_field: EvidenceFieldId,
    authoritative_evidence_reader: EvidenceReaderId,
    authoritative_evidence_digest: EvidenceDigest,
    immutability_proof_id: ImmutabilityProofId,
    invalidation_event: InvalidationEvent,
    proof_test_id: ProofTestId,
    mismatch_detector: MismatchDetectorSpec,
}
```

The declaration serializes the full evidence contract rather than only `kind_id`; this lets a
recorder recompute the content hash and reject a table whose registry metadata was changed or
truncated. `content_sha256` covers every field after `id`, including the exact epoch token and
authoritative evidence digest. `epoch_digest` is the canonical digest of the same exact token.
Thus ids are both content-addressed and session/epoch-scoped.

All fields of `ExpectedAbsentDeclaration` remain private. The registry module exposes only:

```rust
pub fn policy_for(segment: SegmentKey) -> Result<DeclarationPolicy, RegistryError>;
pub(crate) fn evaluate_and_issue(
    segment: SegmentKey,
    epoch: EpochToken,
    reader_output: CompleteAuthoritativeEvidence,
) -> Result<ExpectedAbsentDeclaration, DeclarationIssueError>;
```

`evaluate_and_issue` succeeds only after an exact `Allowed(kind_id)` lookup, full registry
validation, a complete positive read of the authoritative evidence field, recomputation of the
evidence digest, and installation of its mismatch detector. A negative pointer search, bounded
discovery miss, empty cache, timeout, manual list, reader error, incomplete sweep, or zero rows is
not `CompleteAuthoritativeEvidence` and cannot satisfy the signature.

Because `DECLARATION_KINDS` has length zero and every policy is `Forbidden`, Stage B has no
successful call path. This is stronger than convention: any unsafe/test-only constructor is
confined to the registry module's unit tests and is not compiled into production features.

### 4.5 Campaign-identity interface

Stage B consumes this interface without defining its derivation:

```rust
pub enum ReloadBoundary {
    StableSameSave,
    ReloadMayHaveBegun,
}

pub struct CampaignEpochObservation {
    pub boundary: ReloadBoundary,
    pub campaign_identity: Option<CampaignIdentityDigest>,
    pub observation_digest: EvidenceDigest,
}

pub trait CampaignEpochSource {
    fn observe_before_attempt(&self) -> Result<CampaignEpochObservation, IdentityReadError>;
}
```

`observe_before_attempt` is a potentially-blocking operation and is governed by the caller's
`attempt_observation` deadline (sections 5.1 and 5.3): the driver invokes it on an isolated
observation worker under a finite thread-handle wait with zero-time join-or-quarantine, exactly
like every other pre-emission observation. An implementation that is provably a nonblocking
in-memory snapshot read MAY run inline, but the DEADLINE STILL GOVERNS the stage; nothing in this
trait is permitted to wait outside it.

`campaign_identity` is an opaque interface value owned by the campaign-identity design track. It
is stream/session metadata for stitching, not a seventh field smuggled into this G4 token. Before
that track supplies a proved digest, it is `None`; Stage B makes no cross-session sameness claim.
An error or `ReloadMayHaveBegun` still enters the epoch fence immediately. A future provider may
strengthen reload detection without changing the token schema; changing how
`save_epoch_generation` is opened does not change what the generation means.

### 4.6 Frame, declaration-table, seal, and reference records

```rust
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct AttemptId(u64);

pub struct DeclarationTableRecord {
    pub epoch: EpochToken,
    pub declarations: Vec<ExpectedAbsentDeclaration>,
    pub table_sha256: [u8; 32],
}

pub struct ExpectedAbsentRef {
    declaration_id: DeclarationId,
    epoch: EpochToken,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MismatchCheckResult {
    AuthoritativeTokenUnchanged,
    AuthoritativeTokenChanged,
    PositivePresenceAbsent,
    PositivePresenceObserved,
    ReadFailed,
}

pub struct DeclarationCheckRecord {
    pub epoch: EpochToken,
    pub declaration_id: DeclarationId,
    pub attempt_id: AttemptId,
    pub due_attempt_id: AttemptId,
    pub completed_attempt_id: AttemptId,
    pub due_monotonic_ns: u64,
    pub completed_monotonic_ns: u64,
    pub result: MismatchCheckResult,
    pub evidence_digest: EvidenceDigest,
    pub lifecycle_before: DeclarationLifecycleState,
    pub lifecycle_after: DeclarationLifecycleState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeclarationLifecycleState {
    Active,
    Suspended,
    Revoked,
}

pub struct FrameEnvelope {
    pub epoch: EpochToken,
    pub attempt_id: AttemptId,
    pub game_hour: i32,
    pub dispositions: Vec<SegmentDispositionRecord>,
    pub declaration_checks: Vec<DeclarationCheckRecord>,
    pub evidence: Vec<FailureEvidenceBatch>,
}

pub struct EpochSealRecord {
    pub epoch: EpochToken,
    pub last_attempt_id: Option<AttemptId>,
    pub reason: EpochSealReason,
    pub declaration_table_sha256: [u8; 32],
}
```

`last_attempt_id = None` is a legal, distinct state: the declaration table was acknowledged, but
reload uncertainty closed the epoch before its first attempt. H4CR and H4CT both encode this field
with the canonical option tag from section 4.1; zero is not an `AttemptId` sentinel. For
`EpochSealReady` and `EpochSealAck`, the complete H4CT typed payload is:

```text
payload_schema u16 = 1; payload_reserved u16 = 0;
token_sha256 [u8; 32]; declaration_table_sha256 [u8; 32];
h4cr_seal_sequence u64;
last_attempt_presence u8;                    // 0=None, 1=Some
[last_attempt_id u64 only when presence=1];
seal_reason u16; reserved u16 = 0
```

The payload is exactly 81 bytes for `None` and 89 bytes for `Some`, hence the complete 48-byte
H4CT-envelope frame is exactly 129 or 137 bytes respectively. Ready and Ack use the same typed
payload layout and size; the Ack exact-echoes every field. A decoder rejects an unknown tag,
invalid present `AttemptId`, a value occupying the absent branch, truncation, nonzero reserved,
wrong branch length, or trailing bytes before semantic dispatch.

The declaration table is emitted and recorder-acknowledged before any frame referencing it. It is
emitted even when empty, sorted by `DeclarationId`, duplicate-free, and repeated at each
recoverable chunk boundary before that chunk's first reference. Every declaration, frame,
declaration-check result, evidence unit, reference, and seal carries the complete token; no
reference is allowed to inherit one implicitly. All domain records in the frame are covered by
that envelope and attempt id. The frame is invalid if a nested record carries a conflicting hour,
sequence, attempt, or epoch.

Recorder lifecycle state is initialized to `Active` for every declaration only when the exact
declaration table is acknowledged. Table entries are immutable and remain present after
revocation; ids are never deleted, replaced, or reused within an epoch. For every
**frame-producing** attempt the collector emits exactly one `DeclarationCheckRecord` for every
declaration whose recorder state at the start of that attempt is not `Revoked`. This includes
`Suspended` declarations and frame-producing attempts whose segment is `Captured`,
`MeasuredEmpty`, or `Failed` and therefore carries no `ExpectedAbsentRef`. An attempt that emits
**no frame** — a step-0 `attempt_observation`-deadline timeout, or any observation quarantined/
abandoned before step 4 — emits **no** `DeclarationCheckRecord`, because the record's sole carrier
is the `FrameEnvelope`; such an attempt is accounted ONLY in its reason-tagged failed-attempt
ledger and the hour-latch (`observed == attempts + coalesced + in_flight`), consistent with the
step-0 non-poison disposition and the "never reaches this wire partition" rule below. A declaration revoked by its check still has that attempt's one check; only
later attempts omit it.

For both detector arms, `due_attempt_id == attempt_id`; an on-time result also has
`completed_attempt_id == attempt_id`. `AuthoritativeTokenUnchanged` and
`PositivePresenceAbsent` transition `Active|Suspended -> Active`; `ReadFailed` transitions
`Active|Suspended -> Suspended`; `AuthoritativeTokenChanged` and `PositivePresenceObserved`
transition `Active|Suspended -> Revoked`. No result may start from `Revoked`. The recorder compares
`lifecycle_before` to its persisted state and applies only the result-determined
`lifecycle_after`. Its monotonic timestamps measure wall-time delay only and never participate in
identity. Missing, duplicated, late, reordered, impossible-transition, or overflowed check fields
fail the frame and permanently fail the run; they are not repaired from a declaration reference.

Wire decoding first produces module-private `UntrustedDeclarationRecord`,
`UntrustedExpectedAbsentRef`, and `UntrustedDispositionRecord` values. Only
`declaration_registry::decode_and_validate` may turn those into the types above. Production types
do not derive a general-purpose deserializer, so private construction cannot be bypassed by
Serde, a struct update, or a test helper.

Stage E introduces a new top-level stream major whose decoder rejects the old reader/new stream
combination before reading records. New readers may read old streams only as `LegacyUnverified`,
which cannot satisfy the production gate. Unknown record/disposition/invalidation discriminants,
truncated tables, references before declarations, and mixed majors reject.

## 5. Transactional epoch state machine

```text
Detached
  -> Opening(new token; declarations disabled)
  -> AwaitingTokenEcho
  -> PublishingTable
  -> AwaitingDeclarationTableAck
  -> Active
  -> ReloadUncertain(current/overlap attempt Failed; scheduling stopped)
  -> Sealing(old token)
  -> AwaitingRecorderSealAck
  -> Opening(next save_epoch_generation)

Any terminal identity error -> Aborted (run failed; no automatic downgrade)
```

### 5.1 Open

Opening requires a nonzero fresh session nonce, complete process-start identity, fully resolved
modset hash, exe hash, supported schema version, and stable pre-attempt campaign observation. On
EVERY Opening entry it computes one checked absolute monotonic deadline from
`attempt_observation` BEFORE the first pre-token operation. That one unchanged Opening deadline
governs session-nonce CSPRNG, process-handle/creation-FILETIME acquisition, executable image query
and hashing, fully resolved modset/DLC enumeration and content hashing, schema binding, campaign
`observe_before_attempt`, and any declaration-source-generation wait/evaluation. Each operation
that can block runs on an isolated one-operation worker which owns immutable inputs, scratch/result
storage, and any handles it can touch. The driver waits on its thread handle with
`WaitForSingleObject(checked_remaining)`, optionally requests operation-appropriate cooperative or
synchronous-I/O cancellation, makes one zero-time completion check, and either joins a result
observed complete strictly before the deadline or transfers the worker and everything reachable
from it to terminal process-lifetime quarantine under section 5.4. `TerminateThread`, a blocking
hash/query/CSPRNG call on the driver, and reuse of a late result are forbidden. Provably
nonblocking validation over already-owned immutable bytes may run inline, but it still must finish
and pass a final monotonic recheck before publication.

Only after ALL of those operations join and validate before the unchanged Opening deadline may
Opening atomically publish generation 1 (or the checked successor) and the complete token. Expiry,
quarantine, incomplete identity, or a completion first observed at/after the deadline changes
Opening to `Aborted`: it publishes no generation/token/table, emits no Stage-B wire byte, and does
not poison the already-existing transport because no Stage-B wire operation is uncertain. A fresh
product Open attempt may enter Opening with a newly computed deadline, but it re-runs every input
and never reuses a nonce, digest, handle, or late worker result from the aborted entry. If ownership
transfer cannot prove that a straggler is isolated from writable live state, section 5.2's explicit
isolation-failure escalation poisons the transport instead. This is the pre-emission
evaluation-deadline class, not the transport-poison class. Declaration evaluation, if kinds ever
exist, occurs only inside this bounded Opening transaction and only after all required complete
source generations have published.
The recorder is the token authority. Stage B binds its state machine and exact-equality validator
to `StageBControlGate`, the consumer interface over `ControlChannel<ControlIo>` defined in
section 5 of `extractor/documents/DUPLEX_CONTROL_TRANSPORT.md`. The base transport exists, but its
deadline-bearing I/O contract, Win32 adapter, Stage-B payload codecs, timers, and live consumer
wiring remain implementation work; transport availability is not evidence that any semantic
acknowledgement occurred.

### 5.2 H4CT token, table, and seal transactions

All Stage-B control messages use the H4CT v1 envelope and closed direction/kind registry in
sections 2-3 of `extractor/documents/DUPLEX_CONTROL_TRANSPORT.md`: exact magic/version/header and payload
length, nonzero per-attach session id, direction-local consecutive sequence, CRC, zero
flags/reserved fields, and strict typed payload consumption. A framing, direction, sequence,
session, checksum, length, codec, or I/O error poisons the channel before Stage B advances.
Declaration-table, frame, and seal records remain H4CR records in the new Stage-B stream major;
H4CT carries only the permission-to-advance transactions.

Opening is the following transaction:

1. The recorder calls `StageBControlGate::offer_epoch_token`, sends the resulting
   `EpochTokenOffer` (`canonical bytes + SHA-256`), and enters `AwaitingTokenEcho`.
2. The collector exact-decodes all six fields, rejects a noncanonical encoding or unequal live
   binding, and sends one byte-identical `EpochTokenEcho`. Sending is not proof that the recorder
   accepted it.
3. The recorder accepts the echo only through `StageBControlGate::on_token_echo`, comparing both
   exact canonical bytes and digest with the one outstanding offer. It will not validate a table
   ready message until this succeeds.
4. Before the collector starts the H4CR write, the session driver enters the table phase and
   computes its one absolute monotonic deadline. The collector writes the complete
   `DeclarationTableRecord` to H4CR under that deadline, then sends one
   `DeclarationTableReady` naming the exact token hash, table hash, H4CR sequence, declaration
   count, and zero reserved field. It remains paused in `AwaitingDeclarationTableAck`.
5. Under the unchanged table-phase deadline, the recorder drains and frames through that H4CR
   sequence, exact-validates the table, initializes lifecycle state for every declaration, and
   durably admits the table. Only after every operation completed strictly before the deadline may
   `StageBControlGate::on_table_ready` construct `DeclarationTableAck`. The collector exact-checks
   all echoed ready fields. No ack means no referencing frame and no `Active` transition.

Sealing is a second transaction. Before the H4CR seal write, the session driver enters the seal
phase and computes its one absolute monotonic deadline. The collector freezes declaration/lifecycle
state, writes the existing byte-exact `EpochSealRecord` under that deadline, and sends one
`EpochSealReady` naming its exact token/table hashes, H4CR
seal sequence, tagged optional last attempt id, and reason. `None` is required when no attempt
opened before the seal; `Some(id)` must equal the record's validated last attempt exactly. The
recorder uses the unchanged seal-phase deadline to drain through that sequence, validate the seal
against its exact token/table/lifecycle state, durably seal the epoch, and permanently
closes it to later records before `StageBControlGate::on_seal_ready` may construct
`EpochSealAck` with the byte-identical typed payload. The collector exact-checks the option tag,
branch length, and all echoed fields before it may increment the checked
generation and return to Opening. Neither side infers any of these messages from an H4CR prefix,
hash alone, or prior stream history.

The H4CT primitive has no clock, so the Stage-B session driver owns this versioned policy:

```rust
pub struct StageBTimeoutPolicy {
    pub token_echo: NonZeroMonotonicDuration,
    pub declaration_table_ack: NonZeroMonotonicDuration,
    pub attempt_observation: NonZeroMonotonicDuration,
    pub attempt_readers: NonZeroMonotonicDuration,
    pub active_frame_write: NonZeroMonotonicDuration,
    pub active_frame_read: NonZeroMonotonicDuration,
    pub reload_quiesce: NonZeroMonotonicDuration,
    pub epoch_seal_ack: NonZeroMonotonicDuration,
}
```

The policy is immutable for the session and selected by versioned product code, not an
environment variable, user file, machine name, or this developer rig. Its concrete durations are
an enablement input that must be measured on supported weaker PCs; absent, zero, non-monotonic, or
overflowing durations reject Stage-B startup rather than choosing a fallback. Opening and each
active attempt compute separate `attempt_observation` deadlines before the first local operation
or observation they govern, respectively;
step 4 computes `attempt_readers` before its first reader launch/wait. Those local deadlines never
borrow, reset, or extend one another or a transport phase. Entering the token
phase before its first H4CT operation, the table phase before the H4CR table write, and the seal
phase before the H4CR seal write computes one checked absolute monotonic deadline from the
corresponding policy duration. A phase never restarts or extends its deadline after partial
success. Each Active frame record separately computes one checked absolute monotonic write
deadline from `active_frame_write` before submitting its first H4CR byte; partial progress never
restarts that deadline. The `ReloadUncertain` transition similarly computes one checked absolute
monotonic deadline from `reload_quiesce` as part of entering the state, before inspecting, joining,
discarding, or waiting for any in-flight worker. That unchanged reload deadline covers all worker
quiescence and any diagnostic failed-overlap-frame emission; it does not borrow the later
Active-frame or seal deadline.

Every potentially blocking operation in the Stage-B session carries its governing explicit
deadline: local Opening/attempt work uses its entry deadline, transaction operations share the
unchanged phase deadline, and an Active frame write uses its per-record deadline. There is no
deadline-free production overload or unbounded wait hidden behind a semantic gate:

| Operation class | Required deadline-bearing Win32 mechanism |
|---|---|
| H4CT offer, ready, echo, or ack send/receive | `ControlChannel` passes the phase deadline to overlapped `ReadFile`/`WriteFile` in `ControlIo`. |
| Every collector H4CR data-channel write: declaration table, Active frame, and epoch seal | The H4CR handle is opened with `FILE_FLAG_OVERLAPPED`; each `WriteFile` and every partial-write continuation uses its table/seal phase deadline or Active-frame write deadline and the cancellation contract below. |
| Recorder H4CR drain through the named table/seal sequence | Every required pipe `ReadFile` is overlapped and uses the phase deadline. Queue/event handoff waits use `WaitForMultipleObjects` for only the checked remaining time. Post-read mutex, condition-variable, and helper-join operations use the concrete contracts below and the same unchanged phase deadline. |
| Durable table admission and durable epoch seal | The persistence transaction, including each synchronous `FlushFileBuffers`, runs on a dedicated one-operation worker that owns its sink handles and immutable inputs. The driver uses `WaitForSingleObject` for only the checked remaining time. On expiry it calls `CancelSynchronousIo` on that worker, makes one zero-time completion check, and either joins a completed worker or transfers the still-running worker plus every handle/input it can touch to terminal process-lifetime quarantine. `TerminateThread` is forbidden. |
| Opening pre-token inputs: session-nonce CSPRNG, process identity, modset/DLC hashing, executable query/hash, schema binding, campaign observation, and declaration-source waits/evaluation (section 5.1) | Opening entry creates the `attempt_observation` deadline before the first operation. Each potentially blocking operation uses an isolated one-operation worker, finite `WaitForSingleObject(checked_remaining)`, zero-time join-or-quarantine, and no late-result publication. Expiry/quarantine aborts Opening with no generation/token/table or Stage-B wire byte, does not poison transport when isolation succeeds, and permits only a fully fresh Open attempt. |
| Pre-emission attempt observation stage: `observe_before_attempt`, process-identity re-read, exe/modset/schema binding compares, every declaration mismatch detector (section 5.3 steps 1-3) | Attempt entry creates the `attempt_observation` deadline before the first observation. Each potentially blocking observation runs on an isolated worker; the driver uses `WaitForSingleObject` for only the checked remaining time, then zero-time join-or-quarantine. Provably nonblocking in-memory reads may run inline under the same deadline. Expiry/quarantine abandons this attempt before step 4, records every segment `Failed` in reason-tagged failed-attempt accounting, emits no frame, does not poison transport when isolation succeeds, and permits the next attempt. |
| Stable-attempt scheduled readers, sweep-dependency waits, and result joins (section 5.3 step 4) | Step-4 entry creates the `attempt_readers` deadline before any reader launch or wait. Per-reader isolated workers; driver uses finite `WaitForSingleObject(checked_remaining)`, then zero-time join-or-quarantine - the same contract as reload quiescence, applied during the stable attempt. Expiry/quarantine makes that reader's segments `Failed` (never ExpectedAbsent, never stale reuse), does not poison transport when isolation succeeds, and proceeds to the deadline-free nonblocking disposition assembly in step 5. |
| Reload in-flight reader, mismatch-detector, observation, and sweep-generation worker quiescence, diagnostic discard, and failed-overlap-frame write | Entry to `ReloadUncertain` creates the reload deadline before any wait. Each worker thread handle (including sweep-generation workers - section 5.5) is waited with `WaitForSingleObject` for only the checked remaining time and is joined if complete; otherwise its thread handle, output, and every object it can touch move atomically to terminal process-lifetime quarantine, and a quarantined sweep worker's generation is permanently invalidated. A diagnostic discard is an ownership transfer, not a wait. Any failed-frame H4CR write is overlapped under that same reload deadline. Expiry calls `poison_attempt`. |

The drain synchronization contract is concrete and closed:

- Drain-owned in-process mutexes are `CRITICAL_SECTION` or `SRWLOCK` values acquired only with
  `TryEnterCriticalSection` or `TryAcquireSRWLock*`. A failed try performs a monotonic deadline
  check and a bounded nonblocking processor yield before retrying; there is no `EnterCriticalSection`,
  `AcquireSRWLock*`, `WaitForSingleObject(INFINITE)`, or other blocking acquisition in the drain.
  Failure to acquire strictly before the phase deadline calls `poison_attempt`. If a future kernel
  mutex is unavoidable, its acquisition must instead run on an isolated one-operation worker and
  use the helper-join rule below; it cannot be added as a blocking fast path.
- A remaining condition-variable wait uses `SleepConditionVariableCS` or
  `SleepConditionVariableSRW` with a finite per-wait millisecond timeout computed by ceiling the
  checked remaining monotonic duration, clamped to `1..=MAXDWORD-1`, never zero or `INFINITE`.
  Every return, timeout, and spurious wake is followed by a monotonic deadline and predicate
  recheck. Because either API reacquires its associated lock before returning, the entire wait loop
  runs on an isolated drain-condition worker whose state and lock are not shared with the live
  session; the driver also waits on that worker's thread handle with
  `WaitForSingleObject(remaining)`. Thus a stalled lock holder during condition-variable reacquire
  cannot wedge the driver: expiry performs a zero-time thread-completion check and joins or
  quarantines the worker, its lock, predicate state, and thread handle before calling
  `poison_attempt`.
- Every drain helper join is a finite `WaitForSingleObject(thread, remaining)` against the same
  phase deadline. On expiry the driver signals any cooperative stop event, performs one zero-time
  completion check, and either joins or transfers the still-running thread handle, result cell,
  and all reachable handles/state to terminal process-lifetime quarantine, then calls
  `poison_attempt`. The result cell is committed to session state only after a pre-deadline join
  and a final deadline recheck.

`CancelIoEx` cancels only the exact overlapped pipe operation; it does **not** cancel a mutex
acquisition, condition-variable wait, or thread join. `CancelSynchronousIo` applies only to
cancellable synchronous I/O issued by the named target thread; it is not a mutex/CV/join
termination mechanism. `TerminateThread` is forbidden. Bounded wait return plus ownership-safe
join-or-quarantine is the termination mechanism for those synchronization classes.

The semantic gate is called, and an ack constructor becomes reachable, only after every named
prerequisite was observed complete strictly before the unchanged phase deadline. Completion first
observed at or after the deadline is ambiguous and cannot authorize an ack even if the operation
ultimately succeeded. Diagnostics record the policy, phase start/deadline, every blocking
operation's start/completion, cancellation result, quarantine ownership if any, and elapsed wait.

The required Win32 implementation for transport-class named-pipe I/O is **overlapped I/O**, not a
blocking helper thread. Other than the recorder's proven zero-byte `IDLE_EMPTY_REARM` operation
retirement defined exclusively in section 4 of `DUPLEX_CONTROL_TRANSPORT.md`, each named pipe is
opened with `FILE_FLAG_OVERLAPPED` and each `ReadFile`/`WriteFile`
submission owns a fresh
`OVERLAPPED` and manual-reset event whose storage remains live until completion is reaped. The
adapter loops partial transfers under the same absolute deadline, recomputes remaining time from
the session's `QueryPerformanceCounter`-backed monotonic clock, waits only for that remaining time,
and rechecks the absolute deadline before accepting completion. Each operation state and its
handle ownership are heap-stable so expiry cannot create a lifetime race. Expiry or terminal
session/transport caller cancellation calls `CancelIoEx(handle, &overlapped)` and makes one
nonblocking completion check.
If cancellation has completed, the adapter reaps that exact operation before releasing its event,
`OVERLAPPED`, and handle. If completion is not yet observable -- including an `ERROR_NOT_FOUND`
cancel/completion race -- the adapter transfers the handle and operation storage to a
process-lifetime terminal quarantine whose janitor may reap it later; the session call returns
expired/cancelled without waiting again, and the quarantined handle is never reused. A completion
first observed at or after the deadline, a write left partial when the operation terminates, or
any inability to prove the final byte completed before the deadline is ambiguous and terminal.
The idle exception may retain its pipe handle only after the exact operation has been fully reaped
strictly before the liveness deadline as `ERROR_OPERATION_ABORTED` with zero bytes; every other
idle outcome uses this terminal quarantine/no-reuse rule.

The Stage-B session driver owns one `poison_attempt` transition across H4CT, H4CR, drain, and
durability. `ControlChannel` maps send-side and receive-side deadline expiry/cancellation to it;
the H4CR and durability adapters map their expiry, cancellation, late completion, and quarantine
results to that identical transition. It detaches any unreaped operation into terminal quarantine,
permanently closes both channels to new work, suppresses all ack constructors, and permits neither
another operation nor a resend. A future socket backend must use the same
overlapped submission/wait/cancel/reap-or-quarantine contract; a blocking Winsock call is not
conforming.

This blanket terminal-poison rule governs the TRANSPORT-deadline class ONLY: the token, table,
seal, first-frame, per-record read/write, rollover-phase, handshake, and heartbeat deadlines, each of
which bounds a wire operation. For that class a deadline expiry, late complete frame,
cancellation, EOF, broken pipe, short/ambiguous write, or peer-identity change is terminal for the
recording attempt: there is no resend of an uncertain message and no transparent reconnect - the
Stage-B driver permanently poisons the session and rejects a second connection under the same
session id (section 4 of `extractor/documents/DUPLEX_CONTROL_TRANSPORT.md`), because the peer's
state after a timed-out wire operation is unknowable.

The PRE-EMISSION EVALUATION-deadline class (`attempt_observation` on Opening entry and on section
5.3 attempt entry, plus `attempt_readers` on step-4 entry) is DISTINCT and explicitly carved out of
the blanket rule. These deadlines bound purely LOCAL work that has emitted NO Stage-B wire byte,
so expiry leaves no peer state uncertain and does NOT by itself poison the transport session. The
single disposition is state-specific but always non-poison when quarantine proves isolation:
Opening expiry enters `Aborted` without publishing a token; active-attempt observation expiry
abandons that attempt before readers, records every segment `Failed` in reason-tagged
failed-attempt accounting, emits no frame, and permits the next attempt; reader expiry quarantines
that reader, makes only its segments `Failed`, and continues through step-5 assembly and step-6
emission. A late local result can never mutate those decisions. ONLY inability to isolate the
straggler from writable live session state escalates to terminal transport poison. A frame that is
emitted remains governed by `active_frame_write`, whose expiry is transport-class poison.

Recovery never weakens SINGLE-INJECTION. A control failure cannot call the injector or authorize
another DLL load. A fresh control session/product attempt is permitted only after the old module
is positively proven absent; it uses a new CSPRNG session id and both listeners are created before
the one permitted injection. If the game remains alive and module absence is uncertain, the failed
prefix is retained and the result stays visibly failedâ€”there is no reconnect and no reinjection.
These call-graph and fault-injection obligations belong to the live auto-attach integration; the
transport primitive alone does not prove them
(sections 1, 4, and 7 of `extractor/documents/DUPLEX_CONTROL_TRANSPORT.md`).

### 5.3 Active attempt ordering

For each attempt the collector performs this order:

0. on attempt entry, BEFORE step 1, compute one checked absolute monotonic deadline from
   `attempt_observation`. This single unchanged deadline governs steps 1-3 in their entirety:
   the campaign-source `observe_before_attempt` call, the process-identity re-read, every
   binding compare, and every declaration's mismatch detector. Each such observation that can
   block runs on an isolated worker under a finite thread-handle
   `WaitForSingleObject(checked_remaining)` with zero-time join-or-quarantine (the section 5.4
   worker contract); a provably nonblocking in-memory read may run inline but still under the
   deadline. Expiry or any quarantined observation makes the attempt's observation stage
   UNCERTAIN: every scheduled segment is recorded `Failed` in reason-tagged failed-attempt
   accounting, ExpectedAbsent is forbidden, no frame is emitted, and this attempt is abandoned
   before step 4. The transport remains unpoisoned and the next attempt may run once quarantine
   proves the straggler cannot touch writable live state; inability to prove that isolation is the
   sole escalation to `poison_attempt`. The driver never waits past the deadline for a straggler;
1. re-read process identity and pre-attempt campaign/reload signals;
2. compare exe/modset/schema bindings or their immutable live tokens;
3. run every declaration's mismatch detector for this same attempt;
4. if all three stages are complete and stable, compute one checked absolute monotonic
   deadline from `attempt_readers` ON ENTRY TO THIS STEP, then run ordinary scheduled readers
   or apply a valid declaration skip. That single unchanged deadline governs ALL step-4 reader
   work during a STABLE attempt: every scheduled-reader worker launch, its result join, and every
   sweep-dependency wait. It explicitly does NOT govern step-5 disposition assembly. Each reader
   runs on its
   isolated worker; the driver uses finite thread-handle
   `WaitForSingleObject(checked_remaining)` with zero-time join-or-quarantine (the section
   5.4 worker contract - the SAME mechanism, applied here without waiting for a reload
   transition). A reader that expires or is quarantined makes ITS segments `Failed` for this
   attempt (never ExpectedAbsent, never a stale reuse); the attempt itself proceeds to
   disposition finalization with those `Failed` segments rather than wedging - the driver is
   never blocked on a stalled reader with no live deadline;
5. finalize exactly one disposition for every scheduled segment. On `attempt_readers` expiry at
   step 4, every not-yet-decided reader segment is `Failed` and its worker quarantined (above);
   finalization is a BOUNDED NONBLOCKING assembly of the already-decided per-segment
   dispositions (each is a completed read, a valid skip, or a `Failed`/quarantined segment) into
   preallocated frame storage in one finite pass over `SCHEDULED_SEGMENTS`. It performs no I/O,
   allocation, lock acquisition, condition wait, worker launch/join, or other callout and therefore
   has no deadline budget of its own and is not gated by the (possibly-expired)
   `attempt_readers` deadline. Adding any blocking operation to assembly is a contract change that
   requires a new explicit deadline class; it cannot borrow `attempt_readers`. The next deadline is
   step 6's `active_frame_write`, created before emission;
6. before emitting the frame envelope, compute its `active_frame_write` absolute monotonic
   deadline; every H4CR `WriteFile` for that frame must complete strictly before it under section
   5.2's cancel/reap-or-quarantine contract.

Any uncertainty at steps 1-3 prevents ExpectedAbsent for the attempt. It does not reuse the last
successful observation. The `attempt_observation` deadline of step 0, the `attempt_readers`
deadline of step 4, and the `active_frame_write` deadline of step 6 are pairwise separate:
none borrows, extends, or substitutes for another; no pre-emission observation may run under
(or wait for) the reader or emission deadline, and no reader may run under the emission
deadline.

### 5.4 Reload and identity uncertainty

The following are `ReloadMayHaveBegun`: a loading/reload signal; clock rollback/jump that the
campaign source cannot prove continuous; campaign observation change or read error; process
identity mismatch/read error; save-dependent authoritative root invalidation; modset/exe/schema
binding mismatch; or an external capture-continuity break. The collector atomically changes
`Active -> ReloadUncertain` before any more scheduled work and, as part of that entry action,
computes the checked absolute `reload_quiesce` deadline before touching an in-flight worker. The
deadline is never reset per worker. Each ordinary-reader, mismatch-detector, observation, and
SWEEP-GENERATION worker - normatively, EVERY thread the Stage-B session has spawned that can
touch epoch state, reader evidence, or sweep snapshots - is handled
individually: the driver waits on its thread handle with
`WaitForSingleObject(checked_remaining)`, joins and consumes its result only after completion and
a final pre-deadline recheck, or atomically transfers the still-running thread handle, immutable
input, output cell, and every handle/state it can touch to terminal process-lifetime quarantine.
Any cooperative stop request or operation-specific cancellation may be attempted first, but is
never awaited beyond the deadline and is not treated as completion. A diagnostic discard is this
ownership transfer; it cannot detach a worker that still references live session state. Every
worker reserves its quarantine node before launch, and the transfer is a nonblocking
`InterlockedCompareExchange` state transition into that node; reload does not allocate, acquire a
blocking registry mutex, or run a worker-owned destructor on the expiry path.

After all workers are joined or quarantined, the overlapping attempt is emitted `Failed` under
the still-unchanged reload deadline and the H4CR overlapped write contract in section 5.2; it cannot
advance fold or declaration state. Expiry during any worker wait, ownership transfer, or failed
frame write immediately calls the shared `poison_attempt`, suppresses the failed-frame and seal ack
constructors, retains the failed prefix, and returns without another wait. The later seal phase is
created only after reload quiescence and the failed attempt have completed strictly before the
reload deadline; it receives its own `epoch_seal_ack` deadline before the first seal write.

The old declaration table and recorder-visible lifecycle map are frozen, then the H4CR
`EpochSealRecord` / H4CT `EpochSealReady` transaction in section 5.2 runs. No next-epoch record may
be emitted until the collector receives and exact-validates `EpochSealAck`. The next token changes
at least `save_epoch_generation`; process restart also changes `process_start`. Records arriving
for a sealed epoch, or a new declaration arriving before the seal acknowledgement, reject and fail
the run.

### 5.5 Sweep death and restart

Each reader dependency on a sweep carries a complete `SweepGenerationId` in its evidence. Worker
panic, channel closure, coverage loss, or restart invalidates the generation. Dependent segments
are `Failed` from that attempt through the first attempt whose newly published generation is
complete and positively adopted. A restarted thread, partial snapshot, prior generation, or
negative scan cannot issue or revive ExpectedAbsent. If a future declaration's immutability
proof names that generation, generation death/supersession revokes it.

Generation invalidation is a RESULT-VALIDITY rule, not a termination mechanism. On
`ReloadUncertain` entry, every live sweep-generation worker is quiesced under the SAME
section-5.4 `reload_quiesce` contract as ordinary readers: finite thread-handle wait, then
join, or atomic transfer of the still-running thread handle plus every reachable
snapshot/channel/handle it owns to terminal process-lifetime quarantine. A quarantined sweep
worker's generation is permanently invalidated; nothing it later publishes can be adopted, and
the old-epoch state it can still touch is quarantine-owned, never reused by the new epoch.

## 6. Presence sentinel and bounded skew

Every future declaration kind selects exactly one detector in its registry entry.

### 6.1 Revalidation arm

`RevalidateAuthoritativeTokenEveryFrame` reads the authoritative immutable configuration token
before the same frame's disposition decision. A changed value revokes the declaration and enters
the ordinary-reader/fail-run path. An unreadable, torn, late, or incomplete value yields
`Failed`; cached prior evidence is never continued.

### 6.2 Positive-presence arm

The sentinel reader must be structurally independent of the declaration's authoritative evidence
reader: a different positive-presence observation path, not a second call through the same
negative discovery/cache. Registry validation rejects equal reader ids, and the kind's
`proof_test_id` must name the test that demonstrates the two readers do not share the absence
premise; unequal ids alone are necessary but not sufficient.

Production cadence is fixed at every capture attempt:

```text
every_attempts = 1
max_skew_attempts = 1
accepted ExpectedAbsent frames after an unprocessed due sentinel = 0
```

This bounds detection delay to one inter-attempt capture interval plus the already accounted
scheduler wall-time skew. In game-time accounting, a presence transition immediately after
attempt N's sentinel must be detected no later than attempt N+1; coalesced/failed attempts remain
reason-tagged and do not extend an Accepted-ExpectedAbsent lease. The manifest records
`sentinel_due_attempt`, `sentinel_completed_attempt`, and both wall-clock monotonic timestamps so
the bound is mechanically checked rather than asserted.

Per declaration, collector and recorder maintain the same `DeclarationLifecycleState` by the
checked transitions serialized in every required `DeclarationCheckRecord`. A sentinel read error
performs `Active|Suspended -> Suspended`; that attempt and later attempts remain `Failed` until a
complete absent result for the same epoch reactivates it. Suspension does not stop checks. A
positive presence result performs `Active|Suspended -> Revoked` before the ordinary reader runs.
The hit attempt cannot emit ExpectedAbsent. The ordinary reader captures whatever is currently
readable, preserving real data, but the declaration mismatch permanently fails the run.
Revocation is irreversible within the epoch, remains recorder-visible until seal, and suppresses
checks only on later attempts.

## 7. Stage C accounting interface

### 7.1 Dispositions and witnesses

```rust
pub struct SegmentDispositionRecord {
    pub segment: SegmentKey,
    disposition: SegmentDisposition,
}

/// Validated value. The private field prevents callers from naming any variant.
pub struct SegmentDisposition {
    kind: SegmentDispositionKind,
}

/// Module-private: wire decoding uses a separate UntrustedDisposition enum.
enum SegmentDispositionKind {
    Captured(CompletionWitness),
    MeasuredEmpty(CompletenessWitness),
    ExpectedAbsent(ExpectedAbsentRef),
    Failed(FailureSet),
}

pub struct CompletionWitness {
    pub frame_attempt: AttemptId,
    pub source_generation: u64,
    pub contract_proof_id: ProofTestId,
    pub epoch: EpochToken,
    pub digest: EvidenceDigest,
}

pub struct CompletenessWitness {
    pub frame_attempt: AttemptId,
    pub source_generation: u64,
    pub enumeration_begin: u64,
    pub enumeration_end_exclusive: u64,
    pub observed_count: u64,
    pub epoch: EpochToken,
    pub digest: EvidenceDigest,
}

pub struct FailureSet {
    evidence_digests: Vec<EvidenceDigest>, // nonempty, canonical-order, duplicate-free
}
```

The validated disposition has no public fields, variants, general-purpose deserializer, or public
constructor. Captured, MeasuredEmpty, and Failed are returned only by their witness validators.
ExpectedAbsent is returned only by
`declaration_registry::apply_active_declaration`, which requires an opaque active-declaration
lease for the exact segment, token, and attempt plus that attempt's required on-time
`DeclarationCheckRecord`. The recorder must have validated the complete non-revoked check set,
the declaration's exact `lifecycle_before`, a successful unchanged/absent result, and
`lifecycle_after == Active` before the lease can exist. Merely holding or forging a
`DeclarationId` is insufficient. The recorder applies the same registry-owned validation to
untrusted wire values before producing this type.

`FailureSet` must name at least one exact evidence digest present in the same frame; its canonical
order and uniqueness are validated. A missing, foreign-token, or unreferenced failure batch is
fatal rather than silently ignored.

`Captured` means the segment's contract completed, not that rows were nonzero. `MeasuredEmpty`
requires a serialized complete enumeration witness with `observed_count == 0`; a warm cache or
asynchronous sweep is insufficient. Incomplete full-replacement rows may ride the frame for
diagnostics or a separately reviewed upsert fold, but their disposition remains `Failed` and
they do not advance a complete baseline.

For each attempt that reaches step-5 assembly and produces a `FrameEnvelope`:

```text
scheduled_segments
  == captured disjoint-union measured_empty disjoint-union expected_absent
     disjoint-union failed
```

There is exactly one record per scheduled segment, with no missing/duplicate/unknown key. This
partition joins, but does not replace, the hour-latch invariant
`observed == attempts + coalesced + in_flight`.
An attempt abandoned at the step-0 `attempt_observation` deadline never reaches this wire
partition: its reason-tagged failed-attempt ledger records every scheduled segment as failed and
the hour-latch accounts the attempt, but no `SegmentDispositionRecord` or frame is fabricated.

### 7.2 Target exact per-attempt failure evidence

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FailureEvidenceUnit {
    pub epoch: EpochToken,
    pub hour: i32,
    pub domain: DomainId,
    pub segment: SegmentId,
    pub subsource: SubsourceId,
    pub cause_code: CauseCode,
    pub attempt_id: AttemptId,
    pub status_bits: u32,
    pub declaration_id: Option<DeclarationId>,
    pub result: EvidenceResult,
    pub evidence_digest: EvidenceDigest,
}

pub struct FailureEvidenceBatch {
    pub unit: FailureEvidenceUnit,
    pub multiplicity: NonZeroU64,
}
```

This is the G4 target tuple. `SubsourceId`, `CauseCode`, and `EvidenceResult` are closed
enums/newtypes with rejecting decoders. `status_bits` records the bits introduced by that cause,
not the domain-wide OR. Multiple causes in one attempt produce multiple units. A source adapter
may construct this type only when it has a batch-coherent attempt binding; section 7.3 explains
why the shipped Stage-A `so_*` sample path does not yet provide that witness.

The only unit that can support ExpectedAbsent is:

```text
cause_code = SchedulerNotAttemptedImmutablePrerequisite
declaration_id = Some(exact validated declaration)
result = NotAttemptedByDeclaration
status_bits = exact original per-cause failure bits introduced by the scheduler skip
```

The skip does not clear or mask those bits. The declaration-qualified cause is what lets Stage C
account the segment; the raw status still reports reality. All other causes are non-excusable as
ExpectedAbsent. Existing reviewed PARTIAL fold handling remains a separate contract, but it can
neither mint ExpectedAbsent nor turn an incomplete full-replacement observation into Captured.
SOURCE/PARTIAL/BRIDGE/WORKER/CONTROL events without complete mapped units remain fatal under the
applicable completion contract. Combined bits require every first-introduction cause; matching
only one bit is not enough.

### 7.3 Consuming the shipped Stage A `so_*` units

Stage A defines 33 stable origin ids and names
(`extractor/native/compact_contract/src/status_origins.rs:34-124`). Recording happens at the
first-introduction site. Its low-63-bit count and sample-present flag share one swapped atomic,
but the sample payload is a separate atomic
(`extractor/native/compact_contract/src/status_origins.rs:145-198`). A sample writer can store a
new payload after the drain swaps out the old packed batch but before that drain loads the sample;
the old batch then reports the new batch's value. Therefore `_sample` is only a best-effort
last-observed diagnostic, not a batch-coherent or exact per-attempt value. The incidence count
still increments and normally drains in one batch or the next; this caveat is diagnostic precision,
not evidence that the underlying status event or domain record was dropped.

The same code uses `fetch_add` and `wrapping_add`; neither exposes low-63-bit count overflow as the
terminal run failure required by invariant 10. Before any Stage C adapter treats these units as
production evidence it must use a checked/terminal overflow protocol and prove that concurrent
record/drain preserves the committed count/flag batch semantics. Until then an overflow or an
origin whose decision depends on the sample's exact value/batch/attempt cannot construct
`FailureEvidenceUnit`; it yields an unattributed fatal failure and keeps only the raw diagnostic.

The collector currently emits the drain once per frame on reserved probe domain 250
(`extractor/native/collector/src/lib.rs:13185-13233`). The recorder currently reduces those records
into count/last-sample/first-hour/last-hour aggregates
(`extractor/native/recorder_host/src/main.rs:18064-18085`).

Stage C must consume the frame-local probe record before that lossy aggregation. It introduces a
closed `STATUS_ORIGIN_DESCRIPTORS` table, index-aligned with `STATUS_ORIGIN_UNITS`, whose entry
contains:

```rust
pub struct StatusOriginDescriptor {
    pub origin_id: u16,
    pub unit_name: &'static str,
    pub affected_segments: &'static [SegmentKey],
    pub subsource: SubsourceId,
    pub cause_code: CauseCode,
    pub introduced_status_bits: u32,
    pub sample_decoder: SampleDecoderId,
}
```

The adapter may join the probe record's sequence/hour to an active `FrameEnvelope` and copy that
frame's token and attempt id only after the required batch-coherence gate exists. Shared naval
origins explicitly map to both d49 and d50; they are not guessed from the domain-wide OR. After
the overflow/coherence gate, a drain count greater than one is preserved as
`FailureEvidenceBatch::multiplicity`, in the evidence digest as canonical
`(origin_id, count, optional_sample, sequence, affected_segment)`, and in the manifest diagnostic;
it is never silently collapsed to “one event.” The existing last-sample semantics cannot
reconstruct distinct earlier sample values or prove which side of a concurrent drain owns the
value. Current `optional_sample` means best-effort last-observed only, so any origin whose proof
requires sample content must be split or upgraded before Stage C may classify it.

Contract gates require exact set equality between `STATUS_ORIGIN_DESCRIPTORS` and the 33 shipped
ids, exact name equality, valid affected segment keys, and a declared decoder for every sampled
origin. An unmapped id/name, impossible count/sample combination, NO_CLOCK incidence, unmatched
sequence, or ambiguous frame join yields an unattributed fatal failure. Existing manifest
aggregates remain diagnostics only.

With the Stage B empty registry, Stage C can never emit the sole ExpectedAbsent-excusable cause.
Therefore its first deployment adds no tolerance: every failure that was fatal before C1 remains
fatal, while the evidence becomes exact and richer.

### 7.4 Recorder exact-equality algorithm

For each declaration table and frame, the recorder performs these checks in order:

1. decode the supported major/schema and reject unknown or truncated data;
2. compare all six token fields on the table, frame, and any seal with the active epoch using
   derived `Eq`;
3. recompute declaration content/table hashes and require each declaration token to equal the
   table token;
4. require every reference to resolve to a preceding declaration whose full token equals the
   frame token;
5. snapshot the recorder lifecycle map at attempt start and require exactly one on-time
   `DeclarationCheckRecord` for every table declaration whose state is not `Revoked`, including
   `Suspended` declarations and declarations not referenced by any disposition; resolve every id
   against the table, compare its full token/attempt/state transition with the frame, reject any
   extra check, and update lifecycle state only after the whole set is complete;
6. require every evidence batch's full token, hour, and attempt id to equal its frame, preserve its
   nonzero multiplicity in the digest, and reject unreferenced batches;
7. reconcile per-cause introduced bits with the segment's raw status evidence; aggregate OR bits
   alone never pass this step;
8. validate exactly one opaque disposition per scheduled segment and its required witness/evidence;
9. on a seal, require the exact active token, table hash, and final attempt, then permanently reject
   every later record for that token; and
10. only then book the frame into the hour partition.

Any mismatch marks the referenced segment and frame `Failed`, sets a permanent run-gate failure,
and preserves the bytes/diagnostic reason. There is no “closest epoch,” hash-only match, old-token
fallback, or declaration lookup across epochs. A mixed-token frame is never partially accepted.

## 8. Required test battery skeleton

The implementation is not complete until all of these exist and pass.

### 8.1 Registry/constructor contract tests

- `stage_b_declaration_kind_table_is_empty`
- `every_scheduled_segment_has_exactly_one_explicit_forbidden_policy`
- `roster_addition_without_policy_entry_rejects`
- `policy_extra_duplicate_or_unknown_segment_rejects`
- `expected_absent_constructor_is_private` (compile-fail UI test)
- `expected_absent_disposition_variant_and_record_field_are_private` (compile-fail UI test)
- `production_has_no_manual_or_user_absence_loader`
- `d43_d47_d69_are_forbidden_through_every_entry_point`
- `negative_search_empty_cache_timeout_and_incomplete_sweep_cannot_issue`
- `declaration_hash_covers_every_field_and_is_epoch_scoped`
- `partial_tolerance_table_cannot_construct_expected_absent`

### 8.2 Epoch property tests

Use generated event sequences and shrink failures to a minimal trace:

- `prop_reload_may_begin_fails_overlap_seals_then_changes_generation`: inject reload uncertainty
  before, during, and after every attempt phase; no overlapping frame is accepted, and no new
  table precedes old seal acknowledgement.
- `prop_process_restart_or_pid_reuse_never_reuses_token`: vary PID and creation FILETIME
  independently, including recycled PID; missing creation time cannot open an epoch.
- `prop_each_token_component_is_exact`: mutate nonce, PID, creation time, save generation,
  modset hash, exe hash, and schema version one at a time in declarations, frames, references,
  evidence, and seals; every mutation rejects.
- `prop_epoch_counter_never_wraps_or_reopens_sealed_epoch`.
- `prop_late_old_epoch_records_always_fail`.
- `prop_h4ct_token_table_and_seal_transactions_are_exact`: mutate each typed payload field,
  H4CR sequence/digest binding, direction-local H4CT sequence, and transaction order; no bad echo
  or ack advances the collector, and ack constructors remain unreachable before exact durable
  admission/seal.
- `epoch_seal_none_and_some_last_attempt_round_trip_exactly`: round-trip both branches through
  H4CR, `EpochSealReady`, and `EpochSealAck`; require exact 81/89-byte payloads and 129/137-byte
  frames, and reject unknown tags, absent-branch value bytes, truncated present values, invalid
  present ids, wrong branch lengths, and trailing bytes.
- `stage_b_stalled_transport_operation_hits_deadline_and_poisons`: exercise all six H4CT
  token-offer, token-echo, table-ready, table-ack, seal-ready, and seal-ack writes; all H4CR
  declaration-table, Active-frame, and epoch-seal write classes; an H4CR drain `ReadFile`; a
  post-read drain mutex holder that never releases; a condition worker stalled while reacquiring
  its associated lock; a drain helper whose thread handle never signals; and both durability classes
  (table-admission and epoch-seal `FlushFileBuffers`). For pipe cases, the fake overlapped peer
  accepts a strict nonempty prefix where applicable and withholds completion. For durability, an
  isolated worker blocks in the injected flush seam. Every case is held past a short absolute
  monotonic deadline. The call must return `DeadlineExpired` within the test's bounded
  watchdog, record `CancelIoEx` and either completion reaping or terminal-quarantine ownership for
  that exact pipe operation, or record `CancelSynchronousIo` and worker join-or-quarantine for the
  durability operation, or bounded join-or-quarantine ownership for the mutex/CV/helper case;
  permanently poison the semantic session, reject same-session reconnect,
  emit no resend, and prove that no
  failure edge reaches the injector. The isolated-child mutant that routes any one case through
  a deadline-free blocking send, H4CR write/drain, mutex/CV acquisition, helper join, or durability
  wait must exceed the watchdog and
  make this test red;
  the harness kills that child so the suite itself cannot hang. A second fake that withholds the
  cancellation completion must return poisoned through the quarantine branch within the same
  watchdog; waiting indefinitely to reap is a failing mutant.
- `stage_b_pre_emission_deadlines_stall_quarantine_and_do_not_poison`: this is the mandatory
  stall, late-release, and mutation gate for BOTH local policy fields. For
  `attempt_observation`, separately stall every Opening operation (nonce CSPRNG, process-handle/
  creation-FILETIME acquisition, executable query/hash, modset/DLC enumeration/hash, campaign
  observation, and declaration-source wait/evaluation) and every active-attempt observation
  class before it publishes a result. For `attempt_readers`, separately stall a scheduled reader
  result join and a sweep-dependency wait. Each case creates its absolute deadline before the
  first operation, is held past it, returns within an isolated-child watchdog, and transfers the
  worker, result cell, handles, and reachable scratch state to process-lifetime quarantine. The
  Opening cases must enter `Aborted` with no generation/token/table or Stage-B wire byte and no
  transport poison; the active-observation cases must record all segments `Failed`, emit no
  frame, skip step 4, leave transport unpoisoned, and allow the next attempt; the reader cases
  must mark only the affected segments `Failed`, leave transport unpoisoned, complete step 5 as
  the finite nonblocking preallocated-roster pass, and emit under a newly created
  `active_frame_write` deadline. After every timeout the harness releases the worker with a valid
  late result and proves that no token, failed-attempt record, disposition, lifecycle state, or
  later attempt changes. Valid pre-deadline controls must succeed. A mutant that creates either
  deadline after work begins, omits a worker/join bound, borrows or resets another deadline, waits
  indefinitely, publishes a late result, poisons an isolation-safe local timeout, lets a timed-out
  observation reach step 4/emission, reuses stale reader output, or routes disposition assembly
  through the expired `attempt_readers` budget must make this test red.
- `stage_b_reload_stalled_reader_uses_entry_deadline_and_poisons`: enter `ReloadUncertain` while
  one ordinary reader is stalled before publishing its output. Assert that the reload deadline was
  captured before the first worker inspection, the driver returns within its bounded watchdog,
  the reader and all reachable state are quarantine-owned, the overlap attempt cannot advance fold
  or lifecycle state, no seal ack constructor is reachable, and `poison_attempt` records reload
  expiry. A mutant that waits for the reader before creating a deadline, borrows the later
  Active-frame deadline, resets the deadline for each worker, or detaches the reader without owned
  quarantine must make this test red.
- `prop_stage_b_control_recv_timeout_disconnect_and_reconnect_fail_without_reinjection`: fault
  before, during, and after each token/table/seal receive; the explicit receive deadline or
  disconnect poisons, no uncertain message is resent, and no failure edge reaches the injector.
  A fresh attempt remains forbidden until positive old-module absence.

### 8.3 Sweep and sentinel property tests

- `prop_sweep_death_or_restart_fails_dependents_until_new_complete_generation`
- `prop_partial_or_stale_sweep_never_issues_or_reactivates_declaration`
- `prop_late_availability_detected_within_one_attempt`: place presence immediately before/after
  each sentinel boundary; detection is no later than the next attempt, revocation precedes the
  ordinary reader, no hit frame is ExpectedAbsent, real rows ride the stream, and the run fails.
- `prop_sentinel_error_suspends_and_yields_failed_until_complete_absent_result`
- `prop_revalidator_change_and_read_failure_never_continue_absence`
- `prop_presence_and_reload_race_cannot_publish_expected_absent`
- `prop_mismatch_check_token_id_attempt_and_due_completion_are_exact`
- `prop_omitting_any_nonrevoked_declaration_check_rejects_without_reference`: generate active and
  suspended declarations, choose `Captured`, `MeasuredEmpty`, or `Failed` for their segments so
  no disposition references them, first require the complete unmodified frame to validate, then
  delete exactly one required check and require whole-frame and permanent run-gate failure; repeat
  with the check's lifecycle transition corrupted. The valid control case prevents an
  always-rejecting validator from making this property vacuously green.

### 8.4 Stage C accounting properties

- `prop_every_emitted_frame_attempt_partitions_the_exact_segment_roster`
- `prop_captured_requires_contract_completion_not_rows`
- `prop_measured_empty_requires_complete_serialized_witness`
- `prop_torn_partial_worker_and_combined_bits_stay_failed`
- `prop_raw_aggregate_bits_never_grant_tolerance`
- `prop_so_origin_join_preserves_count_sample_hour_attempt_epoch_and_multiplicity`
- `prop_failure_set_rejects_empty_duplicate_missing_and_foreign_batch_digests`
- `prop_shared_naval_origin_maps_exactly_d49_and_d50`
- `prop_unmapped_ambiguous_no_clock_or_late_so_origin_is_fatal`
- `prop_empty_registry_makes_stage_c_gate_behavior_identical`

### 8.5 Mutation and wire tests

The CI mutation job must demonstrate that each mutant makes at least one named test fail:

- drop an attribution unit or collapse/change its batch multiplicity;
- change a cause code, affected segment, subsource, or introduced bit mask;
- add a scheduled segment without a policy or origin mapping;
- change any epoch-token field or compare only its digest;
- remove/delay an epoch seal or accept a post-seal frame;
- make a declaration/disposition constructor, variant, or validated record field public;
- make policy lookup default to Forbidden or allow a user/manual list;
- change an evidence field, reader, proof id, invalidation event, or test id without changing the
  declaration hash;
- skip a sentinel, raise its cadence above one, reuse its cached result, or continue after error;
- replace the recorder's non-revoked table-set check with referenced-only or Active-only checks,
  or accept a lifecycle transition that disagrees with recorder state; the omission property's
  valid control must still pass and each one-check deletion must then make this mutant go red;
- wrap a Stage-A origin count or treat its best-effort sample as batch-coherent evidence;
- mask an aggregate status bit or accept a combined-bit unit with missing causes;
- truncate/reorder the declaration table, reference before declaration, inject an unknown
  discriminant, concatenate restart epochs, or mix stream majors.
- replace the tagged optional seal field with an untagged `u64`, or decode `None` as an attempt;
  `epoch_seal_none_and_some_last_attempt_round_trip_exactly` must go red.
- remove the deadline from any H4CT operation, any H4CR data-channel write, recorder drain, or
  durable admission/seal operation; omit `CancelIoEx`/`CancelSynchronousIo`; omit both completion reaping
  and quarantine ownership; reset a deadline between phase operations; or accept a completion
  first observed at/after deadline. The stalled-transport-operation test must go red.
- remove `attempt_observation` from any Opening input or active-attempt observation; create it
  after the first operation; remove `attempt_readers` from a scheduled reader, dependency wait, or
  result join; reset/borrow either local deadline; omit join-or-quarantine ownership; accept a
  late worker result; poison an isolation-safe local timeout; continue a timed-out observation
  into readers/emission; reuse timed-out reader output; or place the finite nonblocking
  disposition assembly under the expired reader deadline. The mandatory two-local-deadline
  stall/mutation gate must go red.
- replace a drain try-lock with a blocking mutex acquisition, pass zero/`INFINITE` to a condition
  wait, omit the outer join-or-quarantine guard around condition-variable reacquire, block on a
  helper join, or treat `CancelIoEx` as cancelling mutex/CV/join state; the expanded drain-stall
  cases must go red.
- create the reload deadline after joining an in-flight reader, reset it per worker, borrow the
  Active-frame deadline that does not yet exist, or discard a live worker without quarantine;
  `stage_b_reload_stalled_reader_uses_entry_deadline_and_poisons` must go red.

Compatibility tests are `old_reader_rejects_new_major_before_records`,
`new_reader_reads_old_as_legacy_unverified_only`, `truncated_declaration_table_rejects`,
`unknown_disposition_rejects`, `restart_concatenation_requires_seal_and_new_token`, and
`packer_rejects_mixed_majors`.

## 9. Acceptance gates and implementation order

1. Land shared schema, exhaustive segment roster, explicit all-Forbidden policy table, empty kind
   table, and compile-fail constructor tests.
2. Land collector epoch state machine and sentinel framework with no issuable declaration, bound
   to the H4CT primitive with strict tagged-optional seal codecs; `attempt_observation` deadlines
   created before the first Opening input and before each active-attempt observation stage; an
   `attempt_readers` deadline created
   before step-4 work; isolated-worker join-or-quarantine and the section-8.2 stall/mutation gate
   for both local policy fields; their specified non-poison outcomes; deadline-free finite
   nonblocking step-5 assembly; one explicit absolute transport phase
   deadline covering every H4CT operation, every H4CR data-channel write, recorder drain, and
   durable admission/seal operation; a separate reload-entry deadline covering every in-flight
   worker join/discard and failed-overlap emission; concrete bounded mutex/CV/helper-join
   mechanisms; terminal cancel/reap-or-quarantine behavior; and no reconnect.
3. Land recorder exact-token/lifecycle validator and `StageBControlGate` consumer in
   shadow/diagnostic mode; no ack constructor is reachable before its named durable validation,
   and any mismatch is already fatal to its shadow gate.
4. Stage C lands the closed `so_*` descriptor adapter and exact per-attempt accounting only after
   the Stage-A coherence/terminal-overflow gates required by section 7.3. Until those gates exist,
   `so_*` remains diagnostic and ambiguous evidence is fatal. Prove empty-registry gate behavior
   identical over old and generated streams.
5. Only Stage E may enable the new stream major and disposition-aware fold passthrough.
6. A future Stage D declaration-kind proposal is a separate lossless-critical change. It must add
   exactly one complete registry entry, change that segment's explicit policy to `Allowed`, cite
   its code/live dossier and proof id, pass the entire battery, and receive an independent review.

No step may temporarily seed a fake/test production kind. Green with an empty set is achieved by
fixing or correctly failing current sources, never by weakening this registry.

## 10. Self-refutation record

The design was attacked against these prime suspects; the surviving decisions are binding for an
implementation review:

1. **“Empty registry” accidentally conflicts with existing partial tolerances.** Survived by
   making the registries separate and denying ExpectedAbsent construction from the partial table.
2. **A wildcard Forbidden policy would let a new segment bypass review.** Survived by requiring
   exact key-set equality and one explicit policy record per scheduled segment.
3. **Token hashes could hide unequal fields or collision/error handling.** Survived by requiring
   direct six-field equality before digest verification.
4. **PID reuse or a missing creation timestamp could preserve declarations across restart.**
   Survived by making creation FILETIME mandatory and removing pid-only fallback from this path.
5. **Campaign identity was silently invented despite its design track being open.** Survived by
   using an opaque provider interface, keeping it outside the six-field token, and making no
   cross-session sameness claim.
6. **The sentinel could miss one frame or share the same blind spot as discovery.** Survived by
   fixing cadence at every attempt, rejecting stale/due results, requiring a different reader id,
   and permanently failing the run on presence.
7. **Current `so_*` aggregates were mistaken for the required tuple.** Refuted more strongly:
   even the frame-local drain has a separate sample payload atomic, so a concurrent writer can
   make an old batch observe the new batch's sample, and the count operations can wrap without a
   terminal signal. The shipped `_sample` is best-effort last-observed diagnostic data only.
   Stage C fails closed instead of constructing exact evidence until required sample/coherence and
   overflow gates exist; multiplicity remains a target field, not a claim about current precision.
8. **Shared d49/d50 origin ids were ambiguous.** Survived by a closed descriptor with an explicit
   affected-segment set and one logical evidence unit per segment.
9. **A reload recognized late could leak an old declaration into a new save.** Survived by fencing
   on “may have begun,” failing overlap, sealing/acknowledging old epoch first, and rejecting late
   records.
10. **Sweep restart could make absence look newly proven.** Survived by treating death/restart as
    invalidation and requiring a new complete published generation before even ordinary success.
11. **Draft defect: declaration-applied evidence set `status_bits = 0`.** This would have masked
    the existing failure evidence in direct conflict with G4. Fixed: the batch preserves the
    exact original per-cause bits and Stage C accounts them only when the exact declaration and
    scheduler-not-attempted cause match.
12. **Private Rust fields could still be bypassed by generic deserialization.** Fixed by requiring
    separate untrusted wire values and a registry-owned validating decoder; production validated
    types have no general-purpose deserializer.
13. **Stage A counts greater than one had no first-class field.** Fixed by adding the
    `FailureEvidenceBatch` wrapper with nonzero multiplicity; last-sample insufficiency still
    fails closed when per-occurrence values are material to classification.
14. **Draft defect: the public `SegmentDisposition` enum exposed the ExpectedAbsent variant.**
    A private reference payload made casual forgery difficult but did not satisfy the literal
    private-constructor bar. Fixed by making the validated disposition an opaque struct with a
    module-private kind and registry-owned validation path.
15. **Draft defect: `FrameEnvelope` carried `Vec<FailureEvidenceUnit>` and silently lost the
    batch shape.** Fixed by carrying `Vec<FailureEvidenceBatch>`, binding multiplicity into the
    digest, rejecting unreferenced batches, and adding its mutation/property gates.
16. **Mismatch-check timing was prose-only and could not be epoch-compared by the recorder.**
    Fixed by adding explicit token/attempt/timing fields to `DeclarationCheckRecord`.
17. **The design presented a transactional handshake as existing on an inbound-only H4CR pipe.**
    Refuted and replaced with the now-existing H4CT primitive and its `StageBControlGate`
    interface. Token echo, declaration-table ack, and seal ack have exact messages, durable
    prerequisites, deadlines, permanent poisoning, no reconnect, and single-injection recovery
    rules; the live adapter/codecs/consumer wiring remain unimplemented and are not claimed.
18. **A collector could omit a detector on an unreferenced Captured/MeasuredEmpty/Failed attempt.**
    Fixed by making lifecycle recorder-visible and requiring exactly one check for every
    non-revoked table declaration at attempt start, including suspended declarations. Set
    completeness is validated before any transition or frame booking, and the omission property
    deletes one check from each no-reference disposition class.
19. **Draft defect: only `ControlChannel::recv` had a deadline, so a connected peer could stall a
    control write forever.** Fixed by making one checked absolute monotonic phase deadline
    mandatory on every send and receive and choosing a concrete Win32 overlapped
    submit/wait/`CancelIoEx`/reap-or-quarantine implementation. Either-direction expiry or cancellation reaches
    the same terminal poison path; the prefix-then-stall child-process mutant must go red.
20. **Draft defect: `EpochSealRecord` allowed no prior attempt, but H4CT encoded only an untagged
    `u64`.** Fixed with the canonical `u8` option tag, exact `None`/`Some` branch lengths and frame
    sizes, byte-identical Ready/Ack layouts, strict validation, and a no-first-attempt regression
    whose untagged-decoder mutant must go red.
21. **Round-2 defect: only H4CT operations carried deadlines, so an earlier H4CR write, recorder
    drain, or durability operation could strand the phase before a control operation returned.**
    Fixed by starting the table/seal deadline before the first H4CR write, assigning every Active
    frame write its own absolute deadline, carrying the transaction deadline unchanged through
    overlapped H4CR write/drain and worker-isolated durability, and routing every expiry,
    cancellation, late completion, or quarantine result through the same `poison_attempt`
    transition. The stalled-transport-operation mutant covers every transport operation class.
22. **Round-3 defect: reload could join a reader before any applicable frame deadline existed.**
    Fixed by creating an independent absolute reload deadline on entry to `ReloadUncertain`, using
    finite thread-handle waits and per-worker join-or-quarantine ownership, governing the failed
    overlap-frame write with that same deadline, and adding a stalled-reader mutant that must reach
    `poison_attempt` without waiting for the later Active-frame or seal phase.
23. **Round-3 defect: post-read mutex, condition-variable, and helper joins were prohibited from
    being unbounded but had no mechanism.** Fixed by closed try-lock APIs, finite timed
    `SleepConditionVariable*` waits inside an outer bounded worker, finite thread-handle joins, and
    quarantine ownership on every expiry. `CancelIoEx` is explicitly not claimed to cancel these
    synchronization operations, and the drain mutant now stalls each class after a successful
    pipe read.

Independent adversarial review by the orchestrator remains mandatory; this self-refutation is
not that review.

## 11. Fence-external gate finding and proposed patch

The required no-argument census correctly remained blocking, but the required
`python extractor/native/scripts/c0_domain_census.py --selftest` command did not execute a test
suite: it treated `--selftest` as a manifest filename, printed a skip, censused zero sessions,
and exited 0. The cause is mechanical: the usage documents only manifest paths and `main()`
passes every argument directly to `census`
(`extractor/native/scripts/c0_domain_census.py:15-16,119-121`). The same script hardcodes this
rig's OneDrive repo path for its default manifests (`c0_domain_census.py:22-32`), so its
no-argument mode is not worktree-relative. Both fixes are outside this lane's file fence and were
not applied. Exact proposed change:

```diff
--- a/extractor/native/scripts/c0_domain_census.py
+++ b/extractor/native/scripts/c0_domain_census.py
@@
-Usage: python c0_domain_census.py [manifest.json ...]
-       (no args: uses the built-in representative set)
+Usage: python c0_domain_census.py [manifest.json ...]
+       python c0_domain_census.py --selftest
+       (no args: uses the built-in representative set)
@@
-RUNS = (r"C:\\Users\\Example\\OneDrive\\Documents\\Paradox Interactive\\Hearts of Iron IV"
-        r"\\mod\\battle-plan\\extractor\\native\\runs")
+RUNS = os.path.normpath(os.path.join(
+    os.path.dirname(os.path.abspath(__file__)), "..", "runs"))
@@
+def selftest():
+    import tempfile
+
+    assert bits_of((1 << 8) | (1 << 9)) == [8, 9]
+    manifest = {
+        "session_id": "selftest",
+        "compact_stream": {"per_domain": {
+            "dirty": {"domain_id": 998, "record_count": 1,
+                      "data_record_count": 1, "row_count_total": 0,
+                      "world_folded": False, "status_flags_or": 1 << 9},
+            "clean": {"domain_id": 999, "record_count": 1,
+                      "data_record_count": 1, "row_count_total": 1,
+                      "world_folded": True, "status_flags_or": 0},
+        }},
+    }
+    with tempfile.TemporaryDirectory() as directory:
+        path = os.path.join(directory, "manifest.json")
+        with open(path, "w", encoding="utf-8") as handle:
+            json.dump(manifest, handle)
+        sessions, rows = census([path])
+    assert sessions == ["selftest"]
+    assert set(rows) == {"dirty", "clean"}
+    assert rows["dirty"]["bits"] == {9: 1}
+    assert not rows["dirty"]["folded_all"]
+    assert rows["clean"]["bits"] == {} and rows["clean"]["folded_all"]
+    print("c0_domain_census selftest: 6 passed")
+
+
 def main():
-    paths = sys.argv[1:] or DEFAULT_MANIFESTS
+    args = sys.argv[1:]
+    if args == ["--selftest"]:
+        selftest()
+        return
+    if "--selftest" in args:
+        raise SystemExit("--selftest must be used alone")
+    paths = args or DEFAULT_MANIFESTS
```

This proposal changes only developer-tool discovery and its own gate; it does not classify a C0
incidence or alter the Stage B empty registry.
