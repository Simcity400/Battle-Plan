# Campaign identity design

**Purpose:** Define the campaign-identity source, validation rules, wire contract, rollover
semantics, and implementation gates without claiming that the design is already wired.
**Completion status:** See `extractor/documents/MASTER_EXTRACTION_CONTRACT.md`; this document is
the design of record, not a completion ledger.
**Product requirement:** pillar 3 requires an engine-derived identity read from live memory and
stamped into every stream before reload/relaunch stitching can be enabled
(`extractor/documents/PRODUCT_SESSION_SEMANTICS.md:34-39`).

## Verdict

Use the engine-persisted pair **(`game_unique_id`, `game_unique_seed`)** as the version-1
campaign identity source. Preserve both raw components and derive the external `campaign_id`
from their canonical representation. Current-build serializer/deserializer RE proves
`game_unique_seed` as signed `i32[CGameState+0x720]` and `game_unique_id` as the MSVC
`std::string` object at `CGameState+0x728` for executable SHA-256
`870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`; see
[`W15_CAMPAIGN_IDENTITY_STATIC_RE.md`](dossiers/W15_CAMPAIGN_IDENTITY_STATIC_RE.md). The CID1 codec, shared explicit-mode
stateful validator, CFA1 codec, collector producer barrier, durable rollover store, ordered package
publication, replay selection, and recorder-main manifest gate are now statically composed. Pillar
3 is enabled for local/single-machine IdentityV1 recording. Each live observation still fails
closed on build, sentinel, coherence, grammar, CFA1, or rollover uncertainty; there is no static
fallback and no fabricated `Known`. Cross-machine rehost remains a separate disabled capability
until M3 supplies two physical-machine endpoint bundles. Legacy action permits are non-production
and cannot grant either capability.

The current d68 `session` scalar, start date, scenario/bookmark, modset, and human-country
roster are witnesses only. They must never be substituted when either chosen component is
unavailable. Such substitution would merge unrelated campaigns that began from the same
bookmark, or split one campaign after relaunch, hotjoin, tag switch, observer transition, or
modset change.

"Same campaign" means save lineage: copying or renaming a save keeps the same campaign. Two
independent continuations cloned from the same save are branches of that campaign unless the
engine mints a new unique pair. A passive extractor cannot distinguish two byte-identical
cloned lineages from engine state alone; this residual is explicit.

## Existing substrate

### d68 `session_header_live`

d68 is a single-record snapshot rooted at the validated `CGameState` singleton. Its payload
carries five independently presence-badged u32 scalars - controlled-player tag DB id, DLC mask,
campaign start date, save version, and `session` - plus ideology, cosmetic tag, and build banner
(`extractor/native/collector/src/lib.rs:5638-5675`). The reader obtains header scalars at
`CGameState+0xD4`, `+0x4A8`, `+0x58`, and `+0x490`; a bad game-state root produces
`SOURCE_UNAVAILABLE`, and individual bad reads remain absent rather than fabricated
(`extractor/native/collector/src/lib.rs:37520-37543`,
`extractor/native/collector/src/lib.rs:37595-37634`).

The build-profile evidence distinguishes the fields:

- `start_date` at `CGameState+0x4A8` is validated against the save oracle.
- `save_version` at `+0x58` is provisional: one small integer in one save.
- `session` at `+0x490` is provisional: one weak integer in one save.
- the version RVA is a process/build banner, not a save-header identity.

Those labels and offsets are explicit at
`extractor/native/compact_contract/src/build_profile.rs:349-401`. The discovery report says
only one save/live state was available
(`extractor/native/runs/crack_fanout/D3_session_tension/EVIDENCE.md:3-13`) and records `session`
as provisional and requiring a second-save differential
(`extractor/native/runs/crack_fanout/D3_session_tension/EVIDENCE.md:105-114`). Its name and one
matching value do not prove campaign stability or distinctness. The fold validates presence
and retains prior fields last-value-wins; it does not upgrade a field into identity evidence
(`extractor/native/recorder_host/src/world_fold.rs:4923-4983`,
`extractor/native/recorder_host/src/world_fold.rs:5024-5052`).

### d57 `session_multiplayer_metadata_live`

d57's accepted slice is current human-country rows plus pause. Game speed, lobby/persona
identity, player key, MP session slot, host/connected flags, hotjoin, chat, tag-switch, and
session-event hooks are excluded
(`extractor/native/lanes/session_multiplayer_metadata_live/INTEGRATION_PACKET.md:8-17`). Its
schema leaves player key, session slot, host, and connected state `not_sourced`
(`extractor/native/lanes/session_multiplayer_metadata_live/INTEGRATION_PACKET.md:45-70`). The
proof used a one-player SP fixture; multiple humans and lobby-only metadata remain unproved
(`extractor/native/lanes/session_multiplayer_metadata_live/INTEGRATION_PACKET.md:341-350`).

d57 therefore provides change/observer witnesses, not durable identity. A roster hash changes
on hotjoin, drop, tag switch, observer transition, or rehost.

### Save-format candidates

The audited save census contains top-level `game_unique_seed`, `game_unique_id`, and
`multiplayer_random_seed`. Its retained D04 sample reports `game_unique_seed=1026046527`, a
UUID-shaped `game_unique_id="a4b17fed-ed12-4361-9064-fb6576638110"`, and
`multiplayer_random_seed=-1033122218` (`extractor/documents/EXTRACTION_ATLAS.md:166-168`). The
atlas audit calls these unwired one-read scalars off game-state/session roots, not live fields
whose offsets are already established (`extractor/documents/EXTRACTION_ATLAS.md:38-55`).

The save census proves that an engine-persisted UUID-shaped candidate exists. W15's hash-bound
serializer/deserializer route now also proves both current-build member addresses and
representations. It does **not** prove a guarded live read, load stability, cross-client equality,
or new-campaign uniqueness, and no `game_unique_*` field is admitted in the current d68 build
profile (`extractor/native/compact_contract/src/build_profile.rs:349-401`).

The older continuity harness exposes the gap: it prefers a caller-supplied save seed but
otherwise uses a process-instance id stable only for one `hoi4.exe`
(`extractor/live_capture/continuity.py:8-20`, `extractor/live_capture/continuity.py:172-186`).
It cannot distinguish a new campaign loaded into the same process without a live unique seed
(`extractor/live_capture/continuity.py:43-51`). That fallback is not acceptable here.

## Ranked candidates

| Rank | Candidate | Stability and distinctness | Disposition |
| ---: | --- | --- | --- |
| 1 | `game_unique_id` + `game_unique_seed` | Both are top-level save scalars with hash-bound current-build members/representations. Persistence, cross-client equality, and different-new-game distinctness still require measurement. A copied save intentionally remains one lineage. | **Chosen, gated; both mandatory.** |
| 2 | `game_unique_id` alone | Current-build MSVC-string representation is statically proven, but representation/name do not prove load stability or uniqueness. | Retain raw; no v1 activation alone. |
| 3 | `game_unique_seed` alone | Current-build signed-i32 representation is statically proven, but no reload/relaunch pair proves its campaign semantics or collision behavior. | Retain raw; never use alone. |
| 4 | `multiplayer_random_seed` | Save scalar (`extractor/documents/EXTRACTION_ATLAS.md:168`), but may be mutable determinism state. Save-scumming, continued play, or session creation may change it; no evidence settles this. | Diagnostic witness only. |
| 5 | start date + scenario/bookmark + selection hash | Start date has a proven offset (`extractor/native/compact_contract/src/build_profile.rs:363-364`, `extractor/native/compact_contract/src/build_profile.rs:383-384`). Scenario/initial-selection immutable roots are not accepted. Many games share a bookmark; roster changes on hotjoin, tag switch, drop, observer, and rehost. | Compatibility witness; never fallback identity. |
| 6 | d68 `session` at `CGameState+0x490` | One live value matched one save and is explicitly provisional (`extractor/native/runs/crack_fanout/D3_session_tension/EVIDENCE.md:105-114`). It is categorized as a session snapshot (`extractor/native/runs/crack_fanout/D3_session_tension/EVIDENCE.md:257-262`), and one weak u32 is not distinctive (`extractor/native/runs/crack_fanout/D3_session_tension/EVIDENCE.md:286-297`). | Segment/session witness only. |
| 7 | current player/country roster | Breaks on save-scumming with changed selections, hotjoin, tag switch, drop, observer mode, and rehost; common across unrelated games. | Never identity. |
| 8 | process/run id, wall clock, filename, or full-save hash | Process/run values split on relaunch; filename changes on rename; save content changes every save. These are not durable engine campaign fields. | Provenance or segment identity only. |

The legacy design correctly requires retaining raw fingerprint components for later collision
adjudication (`documents/archive/DESIGN_legacy_full_2026-07-02.md:775-780`,
`documents/archive/DESIGN_legacy_full_2026-07-02.md:886-893`). Scenario, modset, roster, start
date, and d68 session remain raw witnesses beside the chosen pair. They stay outside the v1
digest because including them could split one rehosted/observed/updated campaign. An
incompatible witness is a loud conflict requiring adjudication, not a silently different ID.

## Version-1 identity contract

The future reader must independently validate the statically proven current-build members:

1. `game_unique_id`: decode the guarded MSVC `std::string` at `CGameState+0x728`, then require
   canonical lowercase `8-4-4-4-12` ASCII. Malformed size/capacity/pointer state fails closed.
2. `game_unique_seed`: load the exact signed `i32` at `CGameState+0x720` and preserve its signed
   base-10 representation. No unsigned reinterpretation, guessed cast, or truncation is allowed.

Canonical preimage:

```text
ASCII("hoi4-campaign-id-v1\0")
|| ASCII(canonical_game_unique_id)
|| 0x00
|| ASCII(canonical_base10_game_unique_seed)
```

`campaign_id = lowercase_hex(SHA-256(preimage))`.

The digest is an index; UUID and seed remain first-class fields. A digest match with unequal
raw components is corruption/collision and fails closed. Both components are mandatory.
Missing, torn, malformed, or unstable input yields `campaign_identity.state="unavailable"`;
it never produces a partial or fallback ID.

### Read and transition semantics

- Read both fields stably before admitting the first world record, and again after the first
  capture boundary. Both observations must match.
- Re-read at every accepted capture boundary. This detects another save/new game loaded in the
  same process.
- During an unreadable/malformed pair, emit a reason-tagged identity failure and admit no world
  record under the old ID. The failed capture remains accounted.
- A stable unchanged pair continues the stream. A time rollback under that pair starts a new
  segment in the same campaign.
- A stable changed pair closes the stream and starts a new campaign stream/header before any
  record from the new world is admitted.
- One changed component with one unchanged component is `identity_component_conflict`, not an
  automatic new campaign. Preserve both observations and stop automatic stitching.

Identity says which persisted playthrough; hour rollback, process change, and reload evidence
say which segment/branch.

## Stamping contract

### Compact stream header record

The compact stream has no separate file preamble: it is a sequence of H4CR records. The
contract currently defines only record types 1-3
(`extractor/native/compact_contract/src/lib.rs:17-25`). Add a mandatory **first H4CR record in
each logical stream**:

```text
record_type = STREAM_HEADER (new type 4)
domain_id   = 0
sequence    = 0
game_hour_raw = the valid game hour bracketing the stable identity read
payload     = CID1, schema_version=1,
              canonical game_unique_id, canonical game_unique_seed,
              32-byte campaign_id digest, source/build-profile id,
              presence/status flags, raw compatibility witnesses
```

The CID1 v1 payload byte table is now frozen in
`dossiers/W15_CID1_STATIC_CONTRACT.md` and implemented by
`compact_contract::campaign_identity`. Its 96-byte fixed header, bounded suffix
order, flag assignments, and text limits are protocol policy; the UUID and
signed-i32 seed representations are the static engine evidence. This contract is
the active local IdentityV1 wire format; it does not waive any per-observation
runtime guard or the M3 cross-machine capability gate below.

All following records begin at sequence 1. Exactly one header is allowed and it must be first.
Unknown schema, malformed raw fields, digest mismatch, duplicate header, domain 0 on another
record type, or data before the header is a hard decode/pack failure. Legacy streams remain
readable as identity-unavailable and cannot be stitched automatically across sessions.

`Known` is not admitted merely because the pair is canonical. The CID1 decoder must have two
complete observations with matching UUID/seed identity components bracketing admission, a
calendar-valid `game_hour_raw` from the later independently coherent observation, a
non-empty control-free build-profile id, and a SHA-256 recomputed over the complete validated
CID1 payload. The manifest carries that payload hash. If the active exact-build reader fails any
prerequisite, state remains `unavailable` and no digest is authorized for stitching.

### Stateful rollover on the duplex control transport

One injection owns one `control_transport` session and may produce multiple **logical compact
streams**. A logical stream is written to one raw stream/container and therefore still has
exactly one CID1. Loading A then B does not reinject and does not append a second header to A:
recorder and collector cut the physical data-pipe flow at an acknowledged boundary, the host
seals A and opens B, and B begins with CID1.

The normative state machine is:

1. `AwaitInitialIdentity`: no world record may be written. Unreadable, torn, malformed, unstable,
   or incomplete observations use `IdentityFailure` below.
2. `AwaitHeaderAck`: collector writes CID1 as H4CR sequence 0 for logical-stream id 0 and stops.
   The host validates it, recomputes the complete payload hash, durably opens the stream, and
   replies with `FirstCompactFrameAck`. Only a matching ack permits sequence 1.
3. `Streaming`: before every accepted capture boundary, re-read both components. Stable unchanged
   identity admits the next record. Uncertainty enters `IdentityUncertain`; no world record is
   admitted under the old identity.
4. `AwaitRolloverAck`: a stable whole-pair change, or time rollback under the same stable pair,
   sends `RolloverPropose` and stops data writes. Because the control and data pipes have no
   cross-pipe ordering, the host MUST NOT seal the old logical stream or send `RolloverAck` until
   it has received and validated the current stream contiguously through the proposal's advertised
   last H4CR sequence. This drain is deadline-bounded; expiry is terminal failure, never a partial
   seal or acknowledgement. Reject, mismatch, disconnect, or timeout is terminal fail-closed.
5. `AwaitNewHeaderAck`: after acceptance, the host expects the committed CID1 as sequence 0 of the
   next logical stream and validates it before another `FirstCompactFrameAck`; only then may
   sequence 1 be emitted.

Exactly one changed component is not a rollover. It is `identity_component_conflict` only when
the prior and candidate observations are both complete, canonical, independently stable pairs
and exactly one component differs. Missing/malformed/torn components are `unavailable` failures.

Control-frame sequence never resets during an attach. Logical-stream ids start at 0, increment
by exactly one after accepted rollover, and are never reused. H4CR sequence is local to one
logical stream: CID1 is 0 and following records are contiguous from 1. Only the host cuts files;
a reset is legal only after the matching ack and only in the newly opened stream.

The recorder separately binds the first logical-stream id whose semantic `FirstWorld` proof
caused the supervisor to enter `Recording`. Continue may create one or more earlier loading-world
logical streams which have valid CID1/raw evidence but never satisfy `FirstWorld`. Those members
remain in the ordered package set as `pre_world_transient=true` forensic artifacts and are not
allowed to fail the recording-level success gate. Every member at or after the bound
first-recording id remains subject to the ordinary recovery/coverage gates; this exception cannot
hide a failure after recording admission.

All control-payload integers are little-endian. Unknown version/enum/flag, bad length/reserved,
stale nonce, non-contiguous logical id, or hash mismatch poisons the channel. The surrounding
control frame retains the session-id, direction, checksum, maximum-length, and monotonic-sequence
checks already implemented by `recorder_host::control_transport`.

`FirstCompactFrameAck` uses the one canonical 104-byte `CFA1` payload shared with
`DUPLEX_CONTROL_TRANSPORT.md`:

| Offset | Type | Meaning |
| ---: | --- | --- |
| 0 | `[u8;4]` | `CFA1` |
| 4 / 6 | `u16 / u16` | version 1 / total length 104 |
| 8 | `u64` | logical-stream id; `u64::MAX` rejects |
| 16 | `u64` | first complete H4CR sequence |
| 24 | `i32` | calendar-valid H4CR `game_hour_raw` |
| 28 / 30 | `u16 / u16` | H4CR domain id / record type |
| 32 | `u32` | exact complete H4CR header-plus-payload byte length |
| 36 | `u32` | reserved zero |
| 40 | `[u8;32]` | SHA-256 of the exact complete H4CR header plus payload |
| 72 | `[u8;32]` | SHA-256 of the exact H4CR payload bytes |

For identity-v1 the acknowledged first record must be `(sequence, domain, type) = (0, 0, 4)` and
the payload hash must equal the committed CID1 hash. A legacy identity-unavailable stream may ack
only its first sequence-1 known record. There is no second ack for the later PERF/DATA pair that
establishes local `Recording`; the obsolete 48-byte CID-only form is invalid.

`RolloverPropose` payload (`CRP1`, exactly 104 bytes, collector to recorder):

| Offset | Type | Meaning |
| ---: | --- | --- |
| 0 | `[u8;4]` | `CRP1` |
| 4 / 6 | `u16 / u16` | version 1 / length 104 |
| 8 | `u64` | proposed next logical-stream id (current + 1) |
| 16 | `u64` | last admitted H4CR sequence in current stream |
| 24 | `i32` | calendar-valid boundary game hour |
| 28 / 30 | `u16 / u16` | reason (1 whole pair changed; 2 same-pair time rollback) / flags zero |
| 32 | `[u8;32]` | current CID1 payload SHA-256 |
| 64 | `[u8;32]` | proposed next CID1 payload SHA-256 |
| 96 | `u64` | strictly increasing, never-reused proposal nonce |

Implementation warning: CID1's payload hash covers `game_hour_raw`. Therefore
`SamePairTimeRollback` cannot require equality of current and proposed CID1
payload hashes when the rollback changes hour. The rollover implementation must
authenticate same-pair semantics from the parsed UUID/seed (or a separately
carried canonical pair digest) while still binding each distinct complete CID1
payload hash at its own first-record barrier.

`RolloverAck` payload (`CRA1`, exactly 32 bytes, recorder to collector): magic/version/length at
0/4/6, proposed logical id `u64` at 8, echoed nonce `u64` at 16, decision `u16` at 24 (1 accept,
2 reject), error `u16` at 26 (zero iff accepted), reserved-zero `u32` at 28. Acceptance commits
only the boundary and expected new hash. The subsequent `FirstCompactFrameAck` is a second barrier
proving the actual CID1 bytes matched before world data resumes.

#### Identity failure records

Failures use the reliable control stream so they can be recorded before any CID1 and never break
header-first H4CR ordering. Implemented message ids 14/15 remain the authoritative sink setup and
terminal failure reports. The proven-unused adjacent ids are `IdentityFailure = 16` (collector
sender) and `IdentityFailureAck = 17` (recorder sender); both require the distinct
`CAMPAIGN_IDENTITY = 1 << 5` capability.

`IdentityFailure` (`CIF1`) has a 116-byte fixed header followed by exactly
`uuid_len + seed_len + profile_len` raw bytes; malformed raw bytes are preserved:

| Offset | Type | Meaning |
| ---: | --- | --- |
| 0 | `[u8;4]` | `CIF1` |
| 4 / 6 | `u16 / u16` | version 1 / fixed-header length 116 |
| 8 | `u32` | total payload length, checked against all three suffix lengths |
| 12 / 14 | `u16 / u16` | reason / presence-completeness flags |
| 16 | `u64` | current logical id, or `u64::MAX` before initial CID1 |
| 24 | `u64` | strictly increasing attempt id |
| 32 | `i32` | attempted hour; zero iff `hour_valid` is clear |
| 36 / 38 / 40 | `u16` | UUID / seed / profile raw-byte lengths |
| 42 | `u16` | reserved zero |
| 44 | `u64` | last admitted H4CR sequence, or `u64::MAX` if none |
| 52 | `[u8;32]` | current CID1 hash, all zero before binding |
| 84 | `[u8;32]` | SHA-256 of the exact variable raw-byte suffix |

Flags are bit 0 UUID bytes present, bit 1 seed bytes present, bit 2 profile bytes present, bit 3
hour valid, bit 4 first read complete, bit 5 second read complete, and bit 6 reads equal. Unknown
bits reject; clear presence requires zero length and set presence requires non-empty bytes. Reasons
are 1 unreadable root, 2 torn read, 3 UUID missing, 4 seed missing, 5 UUID malformed, 6 seed
malformed, 7 invalid hour, 8 invalid profile, 9 stability incomplete/mismatch, 10 component
transition conflict, and 11 CID1 encode/hash failure. Reason 10 is legal only when both observations
are complete/canonical/stable and exactly one component differs from the current pair.

`IdentityFailureAck` (`CIA1`, exactly 32 bytes) has magic/version/length at 0/4/6, attempt id `u64`
at 8, logical id `u64` at 16, disposition `u16` at 24 (1 durably recorded, 2 rejected), error `u16`
at 26 (zero iff recorded), and reserved-zero `u32` at 28. The host appends the failed capture and
reason to the manifest/gap ledger before acknowledging. Failure control consumes only control
sequence. After an acknowledged uncertain read, stable unchanged identity may resume the current
stream at its next H4CR sequence; a stable pair change must use both rollover barriers.

### One stateful validator for every H4CR walker

`compact_contract::StatefulStreamValidator` now tracks explicit IdentityV1/Legacy mode,
first-record/header state, domain-0 reservation, logical sequence, duplicate CID1, terminal failure,
and retained opaque `ValidatedCid1`. Recorder main validates before dispatch in its dormant
IdentityV1 path, excludes CID1 from ordinary counts, and gates manifest `Known` on that token.

The production complete-stream walkers are now migrated: recorder-host fold/seal/repack and CLI,
the replay decoder's cursor and bounded spool routes, `fold_check`, and compact replay verification
all select mode before reading, drive the shared validator, and consume CID1 before ordinary
counts, boundary state, or `WorldFold::ingest_record`. Portable replay requests and worker receipts
carry that explicit mode. The compact package gate retains the opaque token and exact-joins Known
manifest JSON to `to_manifest_json(resolve_validated_cid1(token))`; it never substitutes d68.
Recorder full-fold and deferred-repack publication perform the same token-derived exact join before
sealing, so a shape-valid but unrelated Known object cannot be republished.

CI must continue to census `H4CR` magic and `parse_compact_record_header` call sites so a new
unmigrated walker cannot silently enter the product. The whole-buffer `WorldFold::ingest_stream`
compatibility helper remains Legacy-only and is not used by the production complete-stream route.
Legacy mode remains explicit and always identity-unavailable.

### Recorder/replay manifest

Recorder main now permits a native-session `known` identity only when explicit IdentityV1
pre-dispatch validation retained `ValidatedCid1`; it cannot derive authority from d68 or a parallel
struct. An ordinary IdentityV1 collector/recorder route exists behind false product latches; the
default active capture path therefore remains Legacy and production output remains
identity-unavailable.
The eventual native-session and embedded replay representation is:

```json
{
  "campaign_identity": {
    "schema": "hoi4.campaign_identity.v1",
    "state": "known",
    "campaign_id": "b28ea1bc4bcea2790b9257208abdc9c27f9985a6e57ef8ce8934d456fcd3be6f",
    "game_unique_id": "a4b17fed-ed12-4361-9064-fb6576638110",
    "game_unique_seed": "1026046527",
    "source": "live_memory",
    "source_build_profile": "profile-x",
    "observed_game_hour_raw": 60759359,
    "stream_header_sha256": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
    "resolution_note": null,
    "witnesses": {
      "start_date_raw": null,
      "session": null,
      "scenario": null,
      "initial_human_country_tags": null,
      "modset_identity": null
    }
  }
}
```

The seed is a JSON string so number conversion cannot alter width/sign. Derive the manifest by
decoding the actual first record, not a parallel struct. Pack/repack validates exact raw-field,
digest, and payload-hash equality. The manifest never overrides stream bytes.

## Validation plan

Static location/representation discovery is complete for the pinned current executable: signed
`i32 +0x720` and MSVC string `+0x728` under `CGameState`. Production performs hash/root admission,
stable-root and repeated-byte bracketing with calendar-valid leading and trailing hour samples,
one coherent guarded `+0x720..+0x747` read, and complete CID1/CFA1 ordering on
every actual session. Any missing guard, hash mismatch, or game patch keeps that session
unavailable and requires fresh static revalidation. The passive matrix remains a conformance
receipt: it records only states the user actually reaches and never drives the client.

Each capture pair retains both streams/manifests, same-tick melted-save values,
build/modset/process provenance, role, and first/last stable identity observations.

| Pair | Arm A -> Arm B | Required result |
| --- | --- | --- |
| S1 | same process, save then reload it | Pair/digest equal; new segment allowed; no uncertain frame assigned to old identity. |
| S2 | reload an older save/save-scum | Pair equal despite lower hour; new segment, same campaign. |
| S3 | close the game, relaunch same save | Pair equal; process and d68 session may differ without changing campaign. |
| S4 | rename/copy save, then load | Pair equal; path/name has no effect. |
| D1 | two new campaigns with identical build, modset, bookmark, and countries | Raw pair and digest differ. A whole-pair alias blocks activation; a repeated single component is recorded and its semantics re-evaluated. |
| D2 | load A then B inside one process | Boundary re-read changes and rolls stream before B's first world record. |
| M1 | host and hotjoin client at same hour | Pair equal; roster difference does not affect digest. |
| M2 | host and observer at same hour | Pair equal; no controlled country does not affect availability. |
| M3 | rehost same save on another machine/player | Pair equal; machine, persona, paths, and host country have no effect. |

A pair passes only when two stable reads bracket admission; live raw values exactly equal the
same-tick save values; expected equality/difference holds; the first record decodes as CID1 and
is hash-bound to the manifest; no data precedes the header or crosses an identity change; and
missing/torn/conflicting reads produce counted failures without a guessed ID. Local recording and
same-machine stitching use those runtime guards directly. M3 alone gates cross-machine rehost.

## Adversarial self-refutation

1. **Names and UUID shape do not prove semantics.** Survives: this design remains gated on
   same-tick and cross-session pairs.
2. **The seed may change while UUID stays stable.** Survives: v1 does not fall back to UUID-only;
   that result blocks v1 and requires a separately reviewed schema.
3. **The UUID may be client/session-local.** Survives: host/hotjoin/observer equality is a gate.
4. **A broader scenario/modset/roster composite looks safer but can split one campaign.**
   Survives: those are raw witnesses with conflict policy, outside the digest.
5. **A copied save can be treated by a user as separate while preserving all engine IDs.**
   Survives as an unavoidable passive-observation limit; copies are one engine lineage with
   branches.
6. **A failed read followed by old-ID reuse can misattribute new-world data.** Survives:
   admission stops during uncertainty and changed identity rolls the stream before resumption.
7. **Manifest metadata can drift from stream truth.** Survives: derive and hash-bind it from the
   first record; mismatch is fatal.

## Implementation map

The design requires coordinated changes in these components:

```diff
extractor/native/compact_contract/src/lib.rs
  CID1 payload/parser, explicit-mode stateful validation, and dependency-neutral CFA1 codec exist.

extractor/native/compact_contract/src/build_profile.rs
  in the d68/top-level CGameState section:
+ DOMAIN_CAMPAIGN_ID_* owner/offset/representation constants
  using the W15-proven i32 +0x720 / MSVC-string +0x728 only after L0 live acceptance;
  never promote static evidence alone into a production fallback.

extractor/native/collector/src/lib.rs
  beside read_compact_session_header and H4CR framing:
  guarded stable readers, sequence-zero CID1 emission, exact CFA1 blocking, and
  fail-closed boundary re-read exist behind inactive product latches.
  Uncertainty/conflict routes through durable CIF1/CIA1, including the
  pre-CID1 startup race, and acknowledged CRP1/CRA1/new-CID1/CFA1 rollover is
  wired into the ordinary product loop.

extractor/native/recorder_host/src/stream_fold.rs
extractor/native/replay_decoder/src/*
+ enforce exactly-one-first CID1; reject data-before-identity for v1;
+ expose legacy identity-unavailable without fallback.

extractor/native/recorder_host/src/main.rs
  in compact_stream_manifest_json/native-session packaging:
  Explicit-mode pre-dispatch validation, opaque ValidatedCid1 retention, Known gating, durable
  logical-stream storage, rollover routing, ordered set packaging, and startup CIF1/CIA1
  arbitration are composed behind inactive latches.

extractor/native/recorder_host/src/world_fold.rs
+ carry campaign/segment provenance without using d68 session/start_date as fallback;
+ keep divergent overlaps and identity-failure spans explicit.

extractor/native/recorder_host/src/stream_fold.rs
extractor/native/replay_decoder/src/*
extractor/native/fold_check/src/*
extractor/native/lanes/compact_replay_verification_gates/src/*
+ replace hard-coded sequence-one/ordinary walkers with explicit-mode
  `StatefulStreamValidator` admission; retain CID1 authority and never dispatch
  the header into `WorldFold` as ordinary data.
```

## Open items

- Static offset/representation RE, CID1 placement/validation, both CFA1 barriers, durable
  CIF1/CIA1 failure accounting, acknowledged rollover, ordered packaging, replay selection,
  and the executable lifecycle owner are complete for the pinned build. There is no
  build-profile fallback.
- `Invoke-Cid1EndpointOwner.ps1` and `Invoke-Cid1PassiveArmRegistration.ps1` own passive S1-S4/D1-D2
  acquisition and registration: exact process/save observations, calendar-sampled barriers, save-oracle
  joins, and raw-H4CR/durable-rollover receipts. The user alone drives HOI4. The quarantined
  `Invoke-Cid1SdActionDriver.ps1` requires explicit legacy opt-in and is not a promotion route.
- M1/M2 use the same passive registrar with two real local process endpoints. M3's frozen relation
  intrinsically requires a second physical machine. Socket or duplicated-transcript loopback is
  not engine lifecycle evidence and remains non-promotional.
- `DOMAIN_CAMPAIGN_ID_LIVE_READER_ENABLED` enables guarded local IdentityV1;
  `DOMAIN_CAMPAIGN_ID_CROSS_MACHINE_REHOST_ENABLED` remains false until M3 passes.
- Independent campaigns cloned from one save cannot be distinguished by passive persisted
  engine identity until the engine changes it.
