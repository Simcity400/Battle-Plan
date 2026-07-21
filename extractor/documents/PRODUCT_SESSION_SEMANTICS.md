# Product session semantics (owner's vision of record — reconciled 2026-07-15)

Owner's sanity-check statement, recorded verbatim in intent (source: session
2026-07-12, direct quote paraphrased):

> You boot it up and then start playing the game. It automatically understands
> which game and which save is attached to it. It saves the progress and spits
> out the replay file at the end. It doesn't matter if you have a reload, or if
> you close the game down and reopen it with an old save — it will try to
> stitch everything together so it looks normal. Settings should be bare
> bones: plug and play, minimal changes to the actual app.

This is the UX contract the extractor is being built toward. The status below separates
offline/code foundations from an activated, live-proven product path. A merged module or synthetic
gate is not evidence that the end-user lifecycle works.

## 1. Boot-and-play (auto-attach) — FOUNDATION BUILT; PRODUCT COMPOSITION OPEN

The recorder has running-process discovery, WMI event sources, singleton/injection-lease logic,
PID/session binding, a journaled coordinator/state machine, and default-deny recording proof. The
new safe production-capture scripts stage a unique DLL copy and define a one-attempt path; they have
not yet completed a real production capture.

OWNER DECISION 2026-07-12: BOTH launch orders must work, symmetrically —
(a) app first: it waits/watches and attaches when the game launches;
(b) game first: booting the app mid-session finds the running game and
attaches immediately. "It always works" regardless of order.

Auto-attach now passes exact attempt/capture identity, a canonical UUID session id, staged-DLL
provenance, production profile, static-bundle inputs, and packaging into the recorder. Remaining
product work is process-lifetime supervision instead of the finite dev worker, graceful stop,
recurring live heartbeat activation, complete Known Folder/OneDrive/`-userdir` discovery,
lifecycle/crash gates, and end-user packaging.
Constraint (standing): must work on arbitrary end-user machines (any install
location, any Steam library, OneDrive-redirected user dirs) — everything
probed at runtime, nothing hardcoded (memory: portability-no-rig-hardcoding).

## 2. Auto-identify game/build/modset/static data — OFFLINE PATH BUILT; LIVE ADMISSION OPEN

The resolver and runtime builder cover executable identity, Paradox user-dir probing, the exact v2
source inventory, ordered active-product DLC/mod definition layers, token/static joins, and all seven
required roles, including `technology_sharing_static`. Asset-only namespaces are explicitly excluded
from definition parsing. Bundle creation and verification pin catalog/store/gate hashes, source
inventory, namespace labels, localisation bytes, and the deliberately unjoined runtime intern-ordinal
policy. Production arguments require this bundle.

This is offline-green, not current-session authority. S9 remains live-blocking until the paused D-0-4
session yields a fresh executable/mod/DLC/load binding, all seven artifacts rebuild and verify with
`unknown=0`, and that exact bundle passes pre-injection, capture, and pre-publication checks. No such
live bundle or capture token exists yet.

## 3. Save/campaign identity — OFFLINE CONTRACT PARTIAL; LIVE BINDING OPEN

The identity resolver, digest, strict manifest grammar, and explicit
known/unavailable/ambiguous/conflict states are built. Hash-bound current-build serializer and
deserializer evidence for executable SHA-256
`870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad` now proves
`game_unique_seed` as signed `i32[CGameState+0x720]` and `game_unique_id` as the MSVC `std::string`
object at `CGameState+0x728`; see
[`W15_CAMPAIGN_IDENTITY_STATIC_RE.md`](dossiers/W15_CAMPAIGN_IDENTITY_STATIC_RE.md). These are
statically proven representations, not live-accepted offsets. The CID1 payload/parser and explicit
IdentityV1/Legacy stateful validator are offline-green. Shared CFA1 validates the exact complete
record and enforces durable write → flush → sync receipt ordering. Recorder main now validates before
dispatch, retains opaque `ValidatedCid1`, excludes CID1 from ordinary counts, and allows manifest
`Known` only through that token (focused 3/3).

Phase B is statically composed end to end: the collector owns the sequence-0/CFA1 barrier and
authenticated `+0x720/+0x728` reader, the recorder owns durable CRP1/CRA1 logical-stream rotation
and ordered package-set publication, and replay requires explicit member selection with exact
descriptor/container joins. The normal product profile nevertheless remains Legacy because the
recorder, collector, and two rollover live-evidence latches are all false. No production identity
or stitching activation is authorized until the owner-run matrix passes.

Activation requires the complete matrix: paused D04 must give two stable, same-hour exact matches to
seed `1026046527` and UUID `a4b17fed-ed12-4361-9064-fb6576638110`; exact-save reload, older-lineage
rollback, process relaunch, and renamed/copied-save arms must preserve the pair; two independently
created otherwise-identical campaigns must differ, and switching measured campaigns in one process
must change identity before any new world row; host/hotjoin, host/observer, and cross-machine rehost
must preserve the pair. Unreadable/torn values, one-component conflict, whole-pair rollover, and
digest-collision states remain fail-closed exactly as P3 defines them in the master contract.

## 4. Reload/relaunch stitching — PLANNER/FORMAT BUILT; PRODUCTION CALL SITE OPEN

- The snapshot container already round-trips SEGMENT-PREFIXED ROLLBACK
  filenames (snapshot/src/container.rs, pack.rs; tests pin both filename
  forms) — the file format was designed for branch/rollback timelines.
- The archived RECORDING_LAYER_COMPLETION_PLAN carries the intended semantics:
  time rollback creates a NEW SEGMENT rather than corrupting the index;
  gaps/rollback spans surfaced, never silently blended.
- The live-capture harness already detects continuity breaks (time_rollback,
  time_jump_forward, pid change) diagnostically.
- Deterministic stitch planning, supersede rules, v2 stitched-container packing/open validation,
  and integration tests exist. The production recorder still has no call site that performs
  pre-fold segmentation, fresh per-segment folds, durable append-ticket ordering, and atomic
  stitched publication across reload/relaunch.
- Stitch activation is additionally blocked on the entire campaign-identity live matrix above; the
  static member proof is not sufficient to join or supersede sessions.
- POLICY CONFIRMED BY OWNER 2026-07-12: whatever comes LATER (in wall-clock
  play order) supersedes whatever came prior on conflict. Canonical example
  (owner's, multiplayer): game crashes; the newest save anyone has is a month
  of in-game time older; the group reloads it and replays that month — the
  REPLAYED month is the main continuity, the pre-crash original of those
  hours is superseded. Superseded spans ARE STILL SAVED (zero-data-loss law:
  retained as rollback segments in the container, visibly superseded) — they
  just don't form the main replay timeline. Playback follows the last-played
  branch end to end.

## 5. Duplex control and heartbeat — PRODUCT ROUTE COMPOSED; ACTIVATION DISABLED

The H4CT envelope, deadline/cancellation channel, peer/capability admission, recurring-heartbeat
state, restrictive overlapped Windows named-pipe adapters, explicit-mode CFA1 codec, durable receipt
contract, real paired H4CR/H4CT server, collector sequence-zero/rollover producer, recorder
filesystem store, ordered publication, and replay selection now exist in the product composition.
Their type-state remains deliberately activation-disabled and cannot mint a product permit.

Activation now requires the owner-run live peer/deadline/cancellation, recurring-liveness,
same-pair rollback, whole-pair change, crash, and failure-boundary evidence. Stage B remains
disabled until that lifecycle passes and its independent promotion authority is recorded.

## 6. Bare-bones settings / plug-and-play — ALIGNED BY DESIGN

Zero-config probing with optional overrides only (e.g. --pdx-user-dir); all
correctness comes from fail-closed evidence, not user tuning. Any future
setting must justify itself against this contract.

Every new session-lifecycle/stitching spec MUST cite this doc; changes to the
vision get recorded here in the same commit.
