# Campaign identity design

**Lane:** `w12g-campaignid-fix`
**Scope:** identity binding and recorder-host protocol design; collector/contract/replay wiring remains
outside this lane.
**Product requirement:** pillar 3 requires an engine-derived identity read from live memory and
stamped into every stream before reload/relaunch stitching can be enabled
(`extractor/documents/PRODUCT_SESSION_SEMANTICS.md:34-39`).

## Verdict

Use the engine-persisted pair **(`game_unique_id`, `game_unique_seed`)** as the version-1
campaign identity source. Preserve both raw components and derive the external `campaign_id`
from their canonical representation. Neither value is currently wired or offset-proven, so
this is a **chosen design, not a validated live identity**. Pillar 3 remains fail-closed until
the validation matrix below passes.

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

This proves that an engine-persisted UUID-shaped candidate exists in saves. It does not prove
its live address, in-memory representation, load stability, or new-campaign uniqueness. No
`game_unique_*` field appears in the current d68 profile
(`extractor/native/compact_contract/src/build_profile.rs:349-401`). The raw D04 melt named by
the discovery report is absent from this worktree, so this lane could not independently
re-read the atlas sample.

The older continuity harness exposes the gap: it prefers a caller-supplied save seed but
otherwise uses a process-instance id stable only for one `hoi4.exe`
(`extractor/live_capture/continuity.py:8-20`, `extractor/live_capture/continuity.py:172-186`).
It cannot distinguish a new campaign loaded into the same process without a live unique seed
(`extractor/live_capture/continuity.py:43-51`). That fallback is not acceptable here.

## Ranked candidates

| Rank | Candidate | Stability and distinctness | Disposition |
| ---: | --- | --- | --- |
| 1 | `game_unique_id` + `game_unique_seed` | Both are top-level save scalars; the first is UUID-shaped (`extractor/documents/EXTRACTION_ATLAS.md:166-167`). Persistence, cross-client equality, and different-new-game distinctness still require measurement. A copied save intentionally remains one lineage. | **Chosen, gated; both mandatory.** |
| 2 | `game_unique_id` alone | Strongest named field and likely high-cardinality, but name/shape is not proof of load stability or uniqueness. | Retain raw; no v1 activation alone. |
| 3 | `game_unique_seed` alone | Could persist, but no reload/relaunch pair proves it; width/sign and collisions remain open. | Retain raw; never use alone. |
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

The future reader must independently validate:

1. `game_unique_id`: canonical lowercase `8-4-4-4-12` ASCII. If memory stores 16 binary bytes,
   byte order is accepted only after exact same-tick save-oracle comparison.
2. `game_unique_seed`: exact engine integer whose signedness and width are established by
   same-tick save comparisons. No guessed cast or truncation is allowed.

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

All following records begin at sequence 1. Exactly one header is allowed and it must be first.
Unknown schema, malformed raw fields, digest mismatch, duplicate header, domain 0 on another
record type, or data before the header is a hard decode/pack failure. Legacy streams remain
readable as identity-unavailable and cannot be stitched automatically across sessions.

`Known` is not admitted merely because the pair is canonical. The CID1 decoder must have two
complete matching observations bracketing admission, a calendar-valid `game_hour_raw`, a
non-empty control-free build-profile id, and a SHA-256 recomputed over the complete validated
CID1 payload. The manifest carries that payload hash. If any prerequisite is absent, including
at the current revision where CID1 does not exist, state remains `unavailable` and no digest is
authorized for stitching.

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

All control-payload integers are little-endian. Unknown version/enum/flag, bad length/reserved,
stale nonce, non-contiguous logical id, or hash mismatch poisons the channel. The surrounding
control frame retains the session-id, direction, checksum, maximum-length, and monotonic-sequence
checks already implemented by `recorder_host::control_transport`.

`FirstCompactFrameAck` payload (`CFA1`, exactly 48 bytes): magic `[u8;4]` at 0, version `u16=1`
at 4, length `u16=48` at 6, logical-stream id `u64` at 8, and recomputed complete CID1 payload
SHA-256 `[u8;32]` at 16.

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

`RolloverAck` payload (`CRA1`, exactly 32 bytes, recorder to collector): magic/version/length at
0/4/6, proposed logical id `u64` at 8, echoed nonce `u64` at 16, decision `u16` at 24 (1 accept,
2 reject), error `u16` at 26 (zero iff accepted), reserved-zero `u32` at 28. Acceptance commits
only the boundary and expected new hash. The subsequent `FirstCompactFrameAck` is a second barrier
proving the actual CID1 bytes matched before world data resumes.

#### Identity failure records

Failures use the reliable control stream so they can be recorded before any CID1 and never break
header-first H4CR ordering. Add closed message kinds `IdentityFailure = 14` (collector sender) and
`IdentityFailureAck = 15` (recorder sender).

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

The earlier claim that every walker used the shared parser was false. Production
`recorder_host/src/main.rs` summarization, recorder fold paths, replay/fold-check tools, and lane
gates contain local H4CR loops/decoders. Before CID1 activation, `compact_contract` must own a
`StatefulStreamValidator` tracking first-record/header state, domain-0 reservation, logical
sequence, duplicate CID1, and explicit legacy mode. Every walker—including `compact_stream`,
production main summarization/packing, recorder-host stream/world folds, `replay_decoder`,
`fold_check`, and every lane-local gate walker—must feed each parsed header to it before payload
dispatch. CI must census `H4CR` magic and `parse_compact_record_header` call sites; any unenumerated
or unmigrated walker blocks CID1-aware/known output. The validator rejects data before CID1, CID1
not at sequence/domain `(0,0)`, duplicate CID1 in one logical stream, any other domain-0 record,
and non-contiguous sequence. Legacy mode is explicit and always identity-unavailable.

### Recorder/replay manifest

`compact_stream_manifest_json` currently records framing, counts, status, coverage, and status
origins (`extractor/native/recorder_host/src/main.rs:17730-17816`). Add this sibling object to
the native-session and embedded replay manifests:

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

First locate both fields from validated `CCurrentGameState`/session-manager roots using
serializer xrefs or same-tick value search. Record RTTI owner, full chain, representation, and
guards; never infer adjacency from save order. Prove exact live/save equality on at least two
same-tick pairs with different UUIDs/seeds, including multiplayer. If either field lacks a
structurally guarded root, pillar 3 remains unavailable. Revalidate after every game patch.

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
missing/torn/conflicting reads produce counted failures without a guessed ID. All S/D/M pairs
are required for production activation.

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

## Outside-fence implementation dossier

This lane may edit only this document. Required future changes, not made here:

```diff
extractor/native/compact_contract/src/lib.rs
  after COMPACT_RECORD_TYPE_DOMAIN_DATA:
+ pub const COMPACT_RECORD_TYPE_STREAM_HEADER: u16 = 4;
  add CID1 payload parser/tests; enforce domain_id=0/type=4 only as first record.

extractor/native/compact_contract/src/build_profile.rs
  in the d68/top-level CGameState section:
+ DOMAIN_CAMPAIGN_ID_* owner/offset/representation constants
  only after same-tick serializer/RTTI proof; never guess from save order.

extractor/native/collector/src/lib.rs
  beside read_compact_session_header and H4CR framing:
+ guarded stable readers for game_unique_id + game_unique_seed;
+ CID1 emission before any frame/domain record;
+ boundary re-read, reason-tagged failure/conflict telemetry, and rollover signal.

extractor/native/recorder_host/src/stream_fold.rs
extractor/native/replay_decoder/src/*
+ enforce exactly-one-first CID1; reject data-before-identity for v1;
+ expose legacy identity-unavailable without fallback.

extractor/native/recorder_host/src/main.rs
  in compact_stream_manifest_json/native-session packaging:
+ decode campaign_identity from first record;
+ write raw fields/digest/witnesses/stream_header_sha256;
+ fail pack/repack on mismatch or identity transition without rollover.

extractor/native/recorder_host/src/world_fold.rs
+ carry campaign/segment provenance without using d68 session/start_date as fallback;
+ keep divergent overlaps and identity-failure spans explicit.
```

## Open items

- No live offset, RTTI owner, representation, or guard is proven for either chosen field.
- No capture pair proves reload/relaunch/hotjoin/observer/rehost stability or distinctness
  across two new campaigns.
- The raw D04 melt and atlas support CSVs named by retained documents are absent here, so their
  scalar samples were not independently re-parsed.
- The prompt names two `extractor/native/runs/production_speed5_*` manifests, but no such
  directories exist here. The current C0 defaults report d57/d68 clean; that does not validate
  identity.
- `c0_domain_census.py --selftest` is not implemented at this revision: `main()` treats every
  argument as a manifest path (`extractor/native/scripts/c0_domain_census.py:119-120`). The
  requested command was run and skipped `--selftest` as a missing path.
- Independent campaigns cloned from one save cannot be distinguished by passive persisted
  engine identity until the engine changes it.
