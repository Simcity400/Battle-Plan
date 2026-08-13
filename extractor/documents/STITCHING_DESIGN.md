> **Read `documents/DECISIONS.md` first.** It is the target authority and outranks this file.
> This file describes the system **as built**.
>
> Superseded here: the new compact-stream records and container-v2 segment layout are not target wire/container edges; the pre-fold stitching and supersede logic remain reusable.

# Reload and relaunch stitching design

**Status:** corrective implementation present for ticket authority, transition
coverage, compatibility gating, repeated-hour ownership, campaign-independent raw
sources, insertion-time parentage, complete stitch-input binding, and strict
container-v2 validation. Live segment formation/wire emission remains open.

**Owner policy:** later-played history supersedes earlier history from the later
branch's cutover hour onward. Superseded history remains in the replay as rollback
data, but is not part of the default playback timeline. This is pillar 4 of
[`documents/DECISIONS.md`](../../documents/DECISIONS.md), including the
owner-confirmed crash/reload example and retention rule
(`documents/DECISIONS.md`).

## 1. Scope and non-negotiable outcomes

This design covers:

- an in-process reload whose first complete post-reload frame goes backward in
  game time;
- a recorder/game relaunch that continues or rolls back the same campaign;
- an exact, deterministic main-timeline selection rule;
- retention and indexing of every superseded snapshot span and every source
  compact stream;
- failure behavior when campaign identity, play order, epoch state, or wire
  compatibility is uncertain.

It does **not** define how campaign identity is derived. That is the responsibility
of [`CAMPAIGN_IDENTITY_DESIGN.md`](CAMPAIGN_IDENTITY_DESIGN.md). That dependency is
not present on this branch at the time of writing; section 4 defines only the
interface stitching consumes. Until a conforming identity provider exists,
cross-session stitching is disabled.

Segment lifecycle metadata, campaign continuity, rollback/time-jump handling,
a new segment on rollback, and visible rollback spans rather than index
corruption are required. This document makes those requirements executable.

### Hard outcomes

1. No raw input is deleted, overwritten, or omitted because it lost branch
   selection. Every accepted session stream is retained as a segment source;
   malformed or uncertain input is retained as recovery evidence.
2. No snapshot from an abandoned future is used to fill a hole in the later-played
   branch.
3. No two sessions are stitched without a verified, equal campaign identity and a
   proven total play order.
4. A rollback is recognized before any record from the rolled-back frame mutates
   the prior segment's fold state.
5. The same ordered source bytes plus the same verified identity/epoch metadata
   produce the same segment graph, snapshot bytes, indexes, and container bytes.

## 2. Established substrate and the missing behavior

The snapshot reader treats an index entry's `file` field as filename authority and
loads through it rather than synthesizing a filename, including segment-prefixed
rollback names (`portable_replay/native/snapshot/src/container.rs:448-476`). The existing
round-trip test packs both an ordinary name and
`s0003.000017050260.snapshot.json`, preserves both names in `index.json`, and loads
the rollback snapshot by hour through that index
(`extractor/native/snapshot/tests/container_roundtrip.rs:20-56` and `:181-212`).
The packer copies payload bytes without reserialization and sorts manifest entries
by path (`portable_replay/native/snapshot/src/pack.rs:1-8` and `:294-358`). Those are the
usable foundations; they are not segment creation.

The current fold is single-timeline:

- `WorldFold` has one persistent accumulator and one `emitted_hours` set
  (`extractor/native/recorder_host/src/world_fold.rs:965-1014` and `:1205-1215`).
- data for an already-finalized hour is counted as late and not applied
  (`extractor/native/recorder_host/src/world_fold.rs:1500-1518` and `:1655-1668`);
- a duplicate `FRAME_END` is skipped and file-backed output always uses the
  unprefixed hour filename (`extractor/native/recorder_host/src/world_fold.rs:2355-2368`
  and `:2380-2401`);
- the stream boundary checker enforces that one open hour closes correctly, but
  it does not establish monotonicity between already-closed hours
  (`extractor/native/recorder_host/src/stream_fold.rs:93-114` and `:156-180`).

Therefore feeding a rollback through the present `WorldFold` cannot implement the
owner policy: overlap records are rejected as already finalized and unique earlier
hours would inherit the abandoned branch's persistent state. Segmentation must be
a pre-fold operation and each segment must own a fresh fold instance.

## 3. Terms and invariants

- **Session:** one recorder attachment to one process-start identity. A PID alone
  is not a process-start identity.
- **Frame:** all records for one `game_hour_raw` ending in its matching
  `FRAME_END`. An incomplete/torn frame never advances visible history.
- **Segment:** a maximal sequence of complete frames within one session and one
  save epoch whose game hours never decrease. Forward gaps do not cut a segment;
  they remain explicit coverage gaps.
- **Cutover hour:** the first observed game hour owned by a segment, even if that
  hour's capture/fold result is Failed. It is not merely the first successfully
  emitted snapshot.
- **Head hour:** the greatest complete observed hour in the segment. A segment
  with no complete frame has no cutover/head and cannot enter a main timeline.
- **Play order:** a strictly increasing campaign-local append ordinal allocated
  by the recorder before accepting a session. It represents the order in which
  sessions were actually played/observed, not filesystem order or file time.
- **Main timeline:** the single default playback history after applying section 6.
- **Rollback data:** any captured snapshot/coverage span excluded from the final
  main timeline by a later segment. It remains addressable through segment indexes.

The following invariants are mandatory:

- segment-local frames are nondecreasing by hour; a decrease starts another
  segment;
- every segment belongs to exactly one verified campaign key and one stream major;
- every complete observed hour has one or more immutable observation records, each
  with exact ticket/segment/epoch owner, evidence id, decoded record-sequence and
  byte bounds, and a disposition (`Snapshot`, `CaptureFailed`, `Coalesced`, or
  `Gap`); the selected segment-local disposition is derived from those records;
- a main-timeline hour has at most one visible snapshot owner;
- a later cutover removes the earlier branch for **all** hours at or after the
  cutover, including an earlier future beyond the later segment's head;
- removed history is reclassified, never deleted;
- holes owned by the later branch remain holes and are never backfilled from a
  superseded segment.

## 4. Campaign identity and play-order interface

Stitching consumes one fail-closed result per verified save epoch from
`CAMPAIGN_IDENTITY_DESIGN.md`:

```text
CampaignIdentityResolution =
    Verified {
        scheme_major: u16,
        campaign_key: opaque bytes,
        evidence_digest: sha256,
    }
  | Missing { reason }
  | Ambiguous { reason, candidate_count }
  | Conflict { reason }
```

Stitching treats `campaign_key` as opaque. It does not inspect save filenames,
paths, timestamps, tags, mod names, scenario names, or game dates to recreate or
strengthen the identity decision. Two epochs may be candidates for one campaign
only when both are `Verified`, their `scheme_major` values match, and their
`campaign_key` bytes are equal. `evidence_digest` is retained for audit but is not
used as the equality key. The provider must re-resolve on every save-epoch
transition; one `hoi4.exe` process may load multiple campaigns, so process/session
identity never pins a campaign for the rest of that process.

After `Verified`, the campaign store atomically allocates a ticket for that
campaign participation:

```text
AppendTicket {
    campaign_key,
    play_order: u64,
    session_nonce,
    process_start_identity,
}
```

Allocation is serialized under a campaign-local lock and durably committed before
the first campaign-bound byte is accepted. Bytes observed while identity is
unresolved remain in a standalone/unbound recovery source; they are not discarded
or prematurely assigned. Distinct tickets claiming one `play_order`, a duplicate
`(play_order,in_session_ordinal)` rank, an absent ledger, a failed lock, or an
imported session lacking its original ticket means **NO STITCH**. Multiple segments
from one session may share a `play_order` only when every byte of their
`AppendTicket` agrees and their in-session ordinals are distinct.
Wall-clock UTC, zip entry order, directory enumeration, modification time, and
lexical filename order never break ties. If an operator later imports sessions in
unknown order, the safe result is separate replays rather than a guessed branch.

`AppendTicket` is the sole ordering/session authority. Every retained segment,
including an empty recovery segment, must carry a ticket and repeat `play_order`,
`session_nonce`, and `process_start_identity` exactly in its duplicated segment
fields/provenance. Any mismatch is corruption and means **NO STITCH**. Ordering is
derived from ticket `play_order` plus the in-session ordinal only after exact
agreement is proven; duplicated fields are never an alternate authority.
Container open independently sorts the serialized ticket ranks, proves that array
position and dense segment id equal that rank, and only then replays insertion-time
parent derivation. Array order alone is never authority.

`session_nonce`, process-start identity, save-epoch generation, modset hash, exe
hash, and schema version also participate in the transactional epoch contract from
the G4 amendments (`extractor/documents/CONFIRMED_COMPLETE_DESIGN.md:236-242`). They
are segment provenance, not substitutes for campaign identity.

## 5. Segment formation

### 5.1 Pre-fold frame staging

The stream driver must stage one logical frame before applying it to `WorldFold`:

1. parse consecutive record sequence numbers;
2. require one hour for all `DOMAIN_DATA` records and a matching `FRAME_END`;
3. bind the frame to its session/epoch token and coverage evidence;
4. compare its hour and epoch with the current segment;
5. select/cut the segment;
6. only then apply the staged records to that segment's `WorldFold`.

An open, mismatched, truncated, or epoch-uncertain frame is retained in the raw
segment source and recorded Failed. It cannot trigger a visible snapshot. This
preserves the existing closed-boundary discipline
(`extractor/native/recorder_host/src/stream_fold.rs:185-213`) without allowing a
partial post-reload frame to poison either branch.

### 5.2 Cut triggers

A new segment is cut before the staged frame when any of these holds:

1. this is the first complete frame of a new session/process-start identity;
2. the verified save-epoch token changes;
3. the collector reports that reload may have begun, even if the new epoch is not
   yet identified; or
4. `frame.hour < current_segment.last_complete_hour`.

Case 3 seals the prior segment immediately. Frames during uncertainty are Failed
and cannot start visible history until a new verified epoch and required baselines
exist. This follows G4's rule that capture stops when reload may have begun, not
only after the new epoch is recognized
(`extractor/documents/CONFIRMED_COMPLETE_DESIGN.md:236-242`).

`frame.hour == last_complete_hour` is not a rollback. Every same-hour observation
is an immutable record carrying its evidence id, exact `AppendTicket` plus
in-session ordinal/epoch owner, decoded record-sequence bounds, decoded byte bounds,
and disposition. The observation with the greatest non-overlapping record sequence
owns the canonical hour, regardless of disposition: success then failure selects a
main gap; failure then success selects the later snapshot. Equal/overlapping source
order, missing evidence, owner disagreement, or a selected disposition that does not
match that last observation rejects the stitch. Vector order, writer completion
order, and last-writer-wins are never authorities. Earlier observations remain
explicit in the per-segment index as well as reconstructible from the retained raw
source. To prevent filename overwrite or current `emitted_hours` rejection, the
implementation must not publish that hour's canonical snapshot until it sees a
greater hour, a segment cut, or segment seal. A forward jump remains in the same
segment and emits explicit `Gap`/`Coalesced` coverage for every unrepresented
requested hour.

### 5.3 Rollback cut transaction

For a backward hour:

1. finish and fsync every previously closed snapshot of the old segment;
2. seal the old segment index and raw-stream extent at the byte immediately before
   the first record of the backward frame;
3. record `stop_reason = time_rollback` and the observed transition
   `(old_hour, new_hour, record_sequence, byte_offset)`;
4. allocate the next deterministic segment ordinal;
5. start a **fresh** `WorldFold`; no countries, states, delta baselines, emitted-hour
   set, or other accumulator is inherited;
6. require the new epoch's baseline/completeness gates before emitting a snapshot;
7. apply the staged backward frame only to the new segment.

If the compact stream cannot prove the new epoch/baseline contract, steps 1-4 still
retain and isolate the bytes, but step 6 fails: the new segment has Failed coverage
and does not publish a fabricated snapshot. It is forbidden to continue applying
the frame to the old fold or to seed a fresh fold from an earlier replay snapshot.

## 6. Exact supersede and visibility algorithm

Let segments be `S[0..n]` in strictly increasing `play_order`, then in increasing
in-session segment ordinal. Each segment has `cutover(S)` and `head(S)`. Empty
recovery segments are excluded from this calculation but retained in the container.

The normative algorithm is:

```text
main = empty ordered coverage map

for S in play order:
    if cutover(S) > main_head + 1:
        require S.transition_coverage exactly covers main_head+1..cutover(S)-1
        require every row is non-Snapshot with reason and evidence id
        insert those rows into main with owner S
    delete from main every hour h where h >= cutover(S)
    for h in cutover(S)..=head(S):
        require S.coverage[h] exists; otherwise reject the stitch
        main[h] = S.coverage[h]
    main_head = head(S)

delete from main every hour h where h > main_head
```

`Snapshot` coverage contributes its segment-prefixed file to the playback index.
`CaptureFailed`, `Coalesced`, and `Gap` contribute an explicit main-timeline gap,
not a snapshot from an older segment.

Segment formation must insert reason-coded `time_jump_forward`, `session_gap`,
`CaptureFailed`, or `Coalesced` coverage before this algorithm runs. For a
cross-session forward transition, the later segment owns a separate normative
`transition_coverage` ledger for every hour after the prior main head and before
its cutover. This is completeness-backed input, not synthesized by the stitcher,
and every row carries a reason/evidence id. Missing, extra, snapshot, or misbounded
transition rows reject the stitch. A missing observed-hour entry is likewise
corrupt input, not a gap kind the stitcher may invent.

Equivalent per-hour rule: for an hour at or before the final `main_head`, choose the
last segment in play order whose cutover is at or before that hour. If the hour is
at or before that segment's head, that segment owns the hour (snapshot or explicit
gap); if it is after that segment's head, the main timeline has a gap. Never fall
back to an earlier segment.

Consequences are intentional:

- `S0=0..100`, then rollback `S1=80..90` yields main `S0:0..79 + S1:80..90`.
  `S0:80..100` is retained rollback data; playback does not resume at old hour 91.
- If `S1` failed to capture hour 85, main hour 85 is a visible gap. `S0:85` remains
  available only through rollback inspection.
- A normal relaunch `S2=91..120` extends the main branch to
  `S0:0..79 + S1:80..90 + S2:91..120`.
- If a later session begins at 110 after the prior head 100, hours 101..109 are an
  explicit cross-session gap; no source is invented.

For auditability, every transition records which later segment first superseded
each normalized earlier span. A segment can therefore have both a visible prefix
and one or more superseded spans; `main`/`rollback` is a span property, not a whole
segment label.

## 7. Container and index layout

Stitched output uses container version 2 and required feature
`timeline_segments_v1` (section 9). Under `payload/<game_id>/` it contains:

```text
index.json
segments.json
timeline.json
s00000000.000016959360.snapshot.json
s00000000.000016959361.snapshot.json
s00000001.000016959300.snapshot.json
segments/s00000000.index.json
segments/s00000001.index.json
segments/s00000001/timeline.json
sessions/p00000000/collector_stream.h4cr.zst
sessions/p00000001/collector_stream.h4cr.zst
```

The canonical snapshot filename is
`s<global-segment-ordinal:08>.<hour_index:012>.snapshot.json`. The prefix prevents
hour collisions while using the index-driven capability already round-tripped by
the snapshot crate. Paths remain game-relative and must pass the existing safe-path
validator.

Each original attachment stream is keyed independently of campaign by the exact
`(session_nonce, process_start_identity, stored_digest, decoded_digest)` tuple and
is retained **once, byte-for-byte**. Its complete source ledger lists every
campaign participation as an exact `AppendTicket`, identity scheme/evidence, and
non-overlapping decoded record-sequence/byte bounds. Each segment references exactly
one of those participations plus segment-local bounds contained by it. Missing,
duplicate, overlapping, foreign-owner, or out-of-participation bindings reject the
stitch.

Before comparison or ledger keying, source paths are lexically normalized with
ASCII case folding, `/` separators, stripped `\\?\` / `\\?\UNC\` prefixes,
and collapsed redundant `.`/`..` components. Drive, UNC server/share, rooted, and
relative namespaces remain distinct. Thus re-observing one physical Windows path
under another spelling and another session/process identity cannot acquire a second
extent ledger; genuinely different canonical paths remain separate.

A source with only one campaign may be embedded under that campaign after every
participation proves the same campaign owner. A source from a process that loads
`A -> B -> A` remains standalone and campaign-independent; the two A segments may
reference their two exact A participation extents, but the whole source may not be
embedded in A or B. Its descriptor/digests are serialized as an external immutable
source reference, while the raw entry itself remains in the standalone source store;
a v2 campaign container rejects a supposedly standalone source that is physically
embedded under its payload. Attempting to embed any cross-campaign source fails closed.
This preserves the entire original stream without moving foreign bytes across
campaign ledgers, copying it once per campaign, or pretending a slice is the whole
source. If the source container is compressed, indexes record both stored-entry and
decoded-stream digests; decoded offsets are never represented as seekable compressed
offsets. Optional derived per-segment streams are non-authoritative caches only.

### 7.1 `index.json`: default playback only

The existing top-level `index.json` remains the compatibility-shaped main playback
index. It contains exactly one entry for each visible `Snapshot` hour, ordered by
ascending hour, and its `file` points at the actual segment-prefixed payload. It
also adds:

```text
index_schema: "stitched-main-v1"
segments_file: "segments.json"
main_head: { segment_id, hour_index }
coverage: ordered visible Gap/CaptureFailed/Coalesced ranges
```

It never lists superseded snapshots and never hides a main coverage gap. The
current reader's `Index` is only a snapshot list
(`portable_replay/native/snapshot/src/lib.rs::Index`; `extractor/native/snapshot`
became a re-export facade over that crate in b0dbf0fe),
which is why version-2 feature gating is required before this richer index is
implemented.

### 7.2 `segments.json`: authoritative branch graph

`segments.json` contains:

- the opaque verified campaign identity scheme/key digest and stitch-input digest;
- the exact play-order ledger digest;
- all segment ids in play order;
- session nonce, process-start identity, epoch token, build/modset/schema bindings;
- cutover/head hours, record-sequence and raw-byte bounds, start/stop reasons;
- parent segment and fork boundary;
- normalized visible and superseded spans, with `superseded_by`;
- each per-segment index/source-reference/timeline path and content digest;
- the ordered main-branch slices and final main head.

The parent of a segment is the segment owning the greatest main-timeline hour below
its cutover at insertion time. A continuation after a gap still records the prior
main tip as parent and records the gap separately. Parentage is explanatory; the
section 6 algorithm, not graph traversal heuristics, is the visibility authority.

### 7.3 Per-segment index

Each `segments/sNNNNNNNN.index.json` contains every complete observed hour and its
selected coverage disposition, every emitted snapshot file/hash, the selection-rule
version, the selected observation id/owner/source bounds, the exact observation
count, and every observation record (not merely a count). It also binds the
campaign-independent source key, exact participation id, original source
path/digests, segment decoded record-sequence/byte bounds, and explicit gaps. It is
immutable once sealed. A
superseded snapshot is therefore still manifest-listed, hash-verified, loadable by
explicit filename, and reconstructible from its retained compact stream.

`timeline.json` is the main selected event timeline. Segment-local event timelines
are retained separately; events from superseded spans are not silently blended into
the main event stream.

### 7.4 Atomic publication

The recorder builds the stitched container beside the last verified output, never
in place. It writes and fsyncs segment payloads/indexes first, derives
`segments.json` and `index.json` from one immutable in-memory model, packs the
container, runs full manifest/hash/reference validation, and only then atomically
publishes it. Failure leaves the prior replay and all standalone/session inputs
untouched. Manifest/index disagreement, an unlisted payload, a missing referenced
entry, or a hash mismatch aborts publication.

## 8. Fold determinism

The deterministic input tuple is:

```text
(stream-major,
 ordered AppendTickets,
 exact compact-stream bytes,
 verified identity results,
 epoch/completeness evidence,
 compatibility proof and exact source bindings,
 fold/schema version,
 publication identity including created_utc,
 compression method and every codec parameter,
 declared container entry-order policy,
 packer codec/library version,
 canonical-writer version)
```

The following rules make output a pure function of that tuple:

Byte determinism is scoped to one declared canonical-writer/packer-codec version.
Changing either defines a different byte domain and must change the bound
publication tuple; it may not be presented as the same deterministic writer.
`created_utc` is an input, never sampled during packing.

`stitch_input_digest` is SHA-256 over one strict canonical representation shared by
planner and reader: game/container/index/required-feature versions, the complete
serialized `segments.json` with only `stitch_input_digest` removed, and every
canonical per-segment index in ticket-proven order. `segments.json` includes the raw
opaque campaign-key bytes as lowercase hex as well as their digest, every ordered
full AppendTicket, identity/source/publication data, and derived branch graph; the
per-segment indexes bind every selected disposition and complete observation record.
All named strings/digests/bounds, fold/schema/writer/codec versions, created time,
compression method, complete non-empty codec-parameter map, and entry-order policy
are mandatory. On open the reader verifies raw-key bytes against the campaign
digest, recomputes both the play-order ledger digest and this full stitch digest,
and rejects any mismatch or non-SHA-256 syntax. A regenerated ordinary manifest
cannot bless a stale or arbitrary stitch claim.

1. Never enumerate files to choose order. Use `play_order`, then stream sequence.
2. Derive global segment ordinals by scanning that order from zero. Do not use
   random UUIDs, PID values, wall clock, mtimes, or thread completion order in ids
   or filenames.
3. Detect cuts on staged logical frames before fold mutation.
4. Give each segment a fresh fold and run the existing canonical serializer.
5. Merge parallel results by `(global_segment_ordinal, hour_index)`. Current
   hour-only sorting (`extractor/native/recorder_host/src/world_fold.rs:882-900`
   and `extractor/native/recorder_host/src/stream_fold.rs:337-380`) is insufficient
   once two segments can contain the same hour.
6. Normalize coverage/superseded spans by ascending hour and coalesce only adjacent
   spans with identical disposition, cause, evidence id, and owner.
7. Canonically serialize indexes and use the declared
   `lexicographic-path-manifest-last-v1` ZIP order. Zip metadata and
   `created_utc` come from recorded deterministic input metadata, not pack
   execution time. Container open compares declared `created_utc` to the manifest
   and the declared entry policy to central-directory order. Until codec parameters
   can be recovered independently from compressed bytes, v2 publication accepts
   only ZIP STORE with the exact profile `{level:none}`; DEFLATE/Zstandard claims
   fail closed even when their method number appears plausible.
8. Reject duplicate ordinals, duplicate record sequences, mixed majors, or two
   snapshot candidates for the same segment/hour rather than tie-breaking by
   iteration order.

Required determinism gates:

- sequential fold versus every supported parallel worker count: identical segment
  graph, per-segment snapshot hashes, main index, and final container SHA-256;
- input chunk sizes and file enumeration order permuted: identical result;
- pack the same immutable model twice: byte-identical container;
- mutate play order, one cutover, one epoch token, or one source byte: the stitch
  input digest and appropriate downstream artifact must change;
- truncate at every record boundary: only sealed complete frames publish; all
  surviving earlier segment bytes/hashes remain identical.

## 9. Wire and version implications

Stitching cannot be an unversioned recorder-only convention. Campaign identity,
transactional epoch tokens, segment boundaries, and per-frame coverage must survive
in the canonical compact stream so an offline fold reaches the same graph.

The G4 amendments require a **new top-level stream major**, old decoders rejecting
before record consumption, unknown discriminants rejecting, session/epoch-scoped
declarations, pack-time mixed-major rejection, and explicit old/new compatibility
tests (`extractor/documents/CONFIRMED_COMPLETE_DESIGN.md:271-278`). Stitching uses
that same major-bump rule. The new major must define, before implementation:

- session/epoch declaration records;
- verified campaign-binding reference/digest;
- append-ticket/play-order binding;
- reload-uncertain and segment-boundary records;
- reason-coded coverage dispositions;
- declaration-before-reference and restart-concatenation behavior.

Legacy streams remain readable by a new decoder in legacy mode, but are never
cross-session stitched and never satisfy the production stitching gate. The packer
rejects a stitched input set containing different stream majors.

The container also bumps from version 1 to version 2 and declares required feature
`timeline_segments_v1`. The current reader caps at container version 2 and fails
closed on unknown required features, while old v1-max readers reject a stitched
archive instead of silently hiding its rollback graph. Snapshot-body
schema need not change solely for stitching; the main/segment index schemas are
versioned independently inside container v2.

The coupling is bidirectional: v2 requires `timeline_segments_v1`, and that feature
is illegal on v1. Opening v2 validates, before returning a `Container`, the strict
stitched index shape, `segments.json`, every referenced segment index, known
coverage discriminants, dense segment ids, unique visible hours, span/reference/
head agreement, segment snapshot hash agreement with the manifest, every mandatory
fold/writer/publication input, exact repeated-observation selection/owner evidence,
raw-source manifest digests and participation bounds, Windows source-path aliases,
embedded-campaign exclusivity, ticket-ranked insertion-time parentage, raw campaign
key/digest agreement, recomputed ledger/stitch digests, manifest-created-time
equality, actual ZIP methods, and actual central-directory order. Critical v2
structures are not accepted merely because permissive raw-JSON fields deserialize.

Compatibility tests must cover:

- old reader/new stitched container: hard reject before snapshot playback;
- new reader/old v1 container: legacy single-segment playback, visibly unverified
  for stitching;
- new reader/new container: main timeline plus explicit rollback access;
- unknown segment/coverage discriminant: reject;
- truncated declaration or segment index: reject;
- mixed stream majors or identity scheme majors: reject;
- restart concatenation without an AppendTicket boundary: reject.

## 10. Failure policy

| Condition | Required result |
|---|---|
| Identity `Missing`, `Ambiguous`, or `Conflict` | Do not stitch. Finalize a separate session replay with the reason. |
| Verified campaign keys differ | Separate replays; never offer date/mod/path similarity as an override. |
| Campaign identity changes inside one process | Seal the old campaign segment; resolve/allocate independently for the new epoch. Never move bytes across campaign ledgers by process continuity. |
| One raw source contains multiple campaigns | Retain it once as standalone and bind each segment to one exact ticketed participation extent. Never embed the source under a campaign; reject any attempted embedded cross-campaign owner. |
| Play-order ticket missing, duplicated, or unprovable | No combined replay; preserve standalone inputs. |
| Same-hour observations have missing/duplicate evidence, overlapping source order, or owner disagreement | Reject; never choose by container/vector position or writer completion order. |
| Any source digest/bound or fold/schema/writer/codec/publication input is missing | Reject before digest/planning; no defaults. |
| Campaign-store lock or append transaction fails | Do not accept bytes into the campaign ledger; record separately/pending. |
| Reload may have begun | Seal old segment immediately; uncertain frames are Failed until a new epoch is verified. |
| Hour goes backward | Cut before applying the frame; fresh fold and baseline gates. |
| Forward time jump | Same segment, explicit gap/coalesce coverage; no invented snapshots. |
| Mixed wire/container/identity major | Reject stitch; keep independently readable compatible inputs. |
| First post-cut frame lacks required baselines | Retain raw bytes and Failed coverage; emit no snapshot from inherited state. |
| Segment/index/hash disagreement | Abort new container publication. |
| Crash during fold/pack | Prior published replay and immutable segment inputs remain; temp output is recovery-only. |
| Later segment has a failed hour that older history captured | Main timeline shows a gap; older snapshot is rollback-only. |
| Later branch ends before an older abandoned future | Main playback ends at the later branch head; older future remains rollback-only. |

Build/modset changes are not campaign-identity heuristics. A verified same-campaign
epoch may cross them only when the target decoder proves per-segment build/modset
and static-catalog compatibility. Until that separate compatibility gate exists or
when it returns uncertain, the sessions remain separate even though campaign
identity matches.

The compatibility provider returns one explicit result per retained participation:
`Proven`, `Missing`, `Conflict`, or `Uncertain`. Only `Proven` may stitch. It binds a
non-empty evidence id, a decoder compatibility class, and the exact source build
SHA-256, modset id, schema version, and static-catalog digest repeated in segment
provenance. Every segment must prove the same target compatibility class; missing,
conflicting, uncertain, or provenance-disagreeing results remain separate.

Identity may be resolved later and a combined replay regenerated from immutable
standalone inputs, but no automatic retroactive merge occurs without the original
verified identity results and AppendTickets.

## 11. Implementation gates

No implementation is complete until all of these pass:

1. rollback inside one stream: `0..100, 80..90` produces two fresh folds, main
   `0..79 + 80..90`, and retains the first `80..100` byte-for-byte;
2. relaunch continuation and relaunch-from-old-save examples from section 6;
3. a failed hour on the later branch remains a main gap despite an older snapshot;
4. persistent fold state cannot cross a cut (domain baselines and tombstones tested);
5. missing/ambiguous/conflicting identity produces two replays and zero combined
   campaign artifact;
6. unknown/duplicate play order fails rather than using timestamps;
7. every snapshot and raw entry, including superseded ones, is manifest-listed and
   hash-verified; tampering any lazy rollback snapshot fails before bytes are exposed,
   matching the existing integrity behavior
   (`extractor/native/snapshot/tests/container_roundtrip.rs:409-425`);
8. sequential/parallel/chunking/repack determinism gates from section 8;
9. G4 old/new/mixed-major battery from section 9;
10. crash injection at every seal/pack/publish phase proves the previous replay and
    every immutable input remain available;
11. property tests generate segment cutovers, gaps, duplicate hours, failures, and
    multiple rollbacks, then prove the section 6 reference algorithm agrees with
    the serialized main index and that every excluded snapshot appears in a
    rollback span;
12. success-then-failure and failure-then-success observations of one hour preserve
    both records and select solely by greatest exact source sequence; equal or
    overlapping sequence evidence rejects;
13. an A -> B -> A process source remains standalone with two exact A
    participation references; marking it embedded under A or B rejects;
14. S0=0..100, S1=80..90, S2=50..60 records S1's parent as S0 at insertion, never
    future S2 inferred from the final graph;
15. mutating every required publication/fold/writer/source input moves the stitch
    digest, and omitting any one rejects both planning and v2 open;
16. stale/arbitrary/non-SHA stitch digests, consistent per-segment evidence tampering,
    forged ledger claims, created-time/method/profile/order divergence, reversed or
    conflicting same-cutover ticket ranks, and drive/UNC path aliases all reject at
    open after the ordinary manifest hashes are regenerated.

The independent reviewer should attack at least these prime suspects: abandoned
future resumption, old-branch gap backfill, state leakage across a fresh fold,
same-hour filename collision, nondeterministic play order, partially visible
segments mislabeled wholesale, container-v1 silent downgrade, and loss of raw bytes
during atomic replacement.

## 12. Self-refutation record

This is the implementer's one-shot sanity check required by the lane workflow. It
does not substitute for the orchestrator's independent adversarial review. Every
finding from the pass is recorded below verbatim, including the findings that did
not require a design change.

1. **“Per-segment `collector_stream.h4cr.zst` entries do not say how an in-session
   rollback can split a compressed stream without losing the exact original source
   bytes or pretending decoded offsets are compressed offsets.”** Confirmed. The
   layout now retains one exact stream per session and segment indexes reference
   decoded bounds and both digests.
2. **“One campaign-identity resolution per process session is unsound because the
   same `hoi4.exe` can return to menu and load a different campaign.”** Confirmed.
   Resolution and ticket allocation are now per verified save epoch; process
   continuity never implies campaign continuity.
3. **“Synthesizing `Gap(unaccounted_segment_hour)` makes a malformed coverage ledger
   look valid and violates the fail-closed accounting law.”** Confirmed. Coverage
   holes now reject stitching; reason-coded gaps must be created upstream.
4. **“The last-observation-wins same-hour rule can overwrite a published filename or
   be defeated by the current `emitted_hours` set.”** Confirmed. Canonical publication
   is delayed until a greater hour/cut/seal. All observations are retained, and the
   canonical owner is the exact ticket-bound observation with greatest decoded
   record sequence regardless of success/failure; completion order never breaks a
   tie, and ambiguous source order rejects.
5. **“The supersede formula might accidentally resume an older future after the new
   branch ends.”** Refuted by the explicit delete-at-cutover rule plus final
   `main_head` truncation; the `0..100, 80..90` acceptance example pins the result.
6. **“A later branch's failed hour might be backfilled from an older captured hour.”**
   Refuted by segment ownership applying to every hour in its span and by the
   explicit failed-hour gate in sections 6 and 11.
7. **“Whole-segment `main`/`rollback` labels would lose a segment's still-visible
   prefix.”** Refuted by normalized span-level visibility and `superseded_by`
   metadata; no whole-segment disposition is authoritative.
8. **“Equal campaign keys could still be undecodable across a game/mod transition.”**
   Confirmed. Identity equality is now separated from a fail-closed per-segment
   build/modset/static-catalog compatibility gate.

## ZIP-container hardening residual (2026-07-14 — STOP decision at fix6)

The v2 container open path was adversarially hardened across fix3→fix6 (6 gate rounds):
local-vs-central header cross-check, unconditional whole-archive payload **CRC-32** before any
manifest field is trusted, disk-number/multi-disk rejection, Zip64 extra-field bounds + duplicate
rejection, ordinary-vs-Zip64 EOCD cross-check, and CD-end-at-EOCD (no CD-in-comment). Each gate
round closed its findings and surfaced new **same-class ZIP-format edge cases** — an open-ended
regress with diminishing returns. Per orchestrator ruling this hardening was **stopped at fix6**.

**Accepted residual (documented, NOT a production/data-loss defect):**
- **Lazy payload verification (by design).** Open validates CRC-32 over all payloads but does not
  eagerly re-derive every manifest **SHA-256** at open; a payload whose bytes AND CRC were both
  rewritten passes open, and is caught by the fail-closed **read-time** SHA check
  (`read_entry_bytes`/`verify_all`) when the payload is actually read. Every payload that a replay
  consumes is read (and thus verified); a never-read payload is inert. Eager verify-all-at-open was
  deliberately not adopted (it defeats lazy loading; `container_roundtrip.rs:414` asserts the lazy
  contract).
- **Two Zip64 micro-conformance edge cases:** the Zip64 EOCD `record_size` is checked `>= 44` but
  not bound to `zip64_eocd_offset + 12 + record_size == locator_offset`; a Zip64 extra-field body
  length is not required to equal the exact sentinel-member width.

**Threat model:** all residuals require a hostile **same-user** process crafting a malformed
container in the output directory on the user's own single-user machine. **Scope:** the stitching
*feature* itself is future/unbuilt; only its container loader exists. If/when stitching is built and
the threat model tightens, revisit (add read-time-equivalent SHA gating at open or the exact Zip64
extents). Not merge-blocking.
