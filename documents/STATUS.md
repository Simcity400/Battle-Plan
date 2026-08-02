# Project status

> **Read `DECISIONS.md` first.** It states what the product is supposed to be and outranks
> this page and every specification here. This page describes the system **as built**.
> Acquisition, the wire format and the container are being replaced, so do not read what
> follows as the target.
>
> Two things below are already superseded. The pause-owning acquisition path has been
> replaced in the in-flight Extractor Layer by an injected,
> zero-semantic-state-mutation,
> tick-bracketed scheduler; the working tree is not
> production authority until central validation and a real speed-five passive capture issue
> fresh receipts. And the product was built for one owner's machine, whereas it must now work
> for any player, on any mod set, with any DLC.

`DECISIONS.md` is intent. This page is implementation. `SEAM_CONTRACTS.md` is what each half
is waiting on from the other. An agent landing cold reads them in that order.

**Pipeline as built:**
`hoi4.exe → injected DLL collector → typed compact records → Rust recorder → .battleplan → replay app`.
Doctrine is fail-closed: anything the replay cannot reproduce exactly surfaces as a typed
blocker, never a silent approximation.

## Extractor — recording half

`extractor/native/domain_registry.json` is code-authoritative and registers **86 domains**:
49 production-complete, 27 existing, 2 validated slices, 1 partial slice, 5 reserved, and 2
infrastructure. 79 are schedulable.

All 14 gameplay areas extract at base slices. Named residual functionality remains in market,
naval, combat, intel, supply and event-hook coverage. `extractor/documents/MASTER_EXTRACTION_CONTRACT.md`
holds the completion table and `extractor/documents/RE_REMAINING.md` the RE ledger. Per-domain
knowledge is condensed into `extractor/documents/domains/`.

The product implementation has landed: the no-argument desktop route enters auto-attach, duplex
activation is enabled, campaign identity works same-machine, and durable append-ticket stitching
is called from production. **That describes implementation, not a current receipt** — central
validation has not been re-run against the present worktree. Rehosting campaign identity on a
second physical machine is the one genuine external dependency.

The Extractor rebuild treats injection and hooks separately from client
control. The production collector remains injected and may retain the small,
evidence-backed hook allowlist, but it may not write semantic engine state. The
canonical recorder, passive capture driver, package-byte denylist, retained
observer/no-write markers, and read-only speed-five witness form the intended
interaction closure. Evidence client-driving is separately named and excluded
from the package. This paragraph records the implementation boundary only: no
current Release receipt or genuine speed-five capture is claimed here.

A paused game is a recordable state. the game auto-pauses constantly in ordinary
play — events, decisions, focus completion, war declarations, peace conferences
— so the capture harness treats a pause as a classified observation rather than
as client drift. The zero-client-writes guarantee rests on game-speed invariance
alone, which the engine's pause path provably never touches; clock movement is a
budgeted rate observation whose sustained absence is reported as a hang under its
own name. `extractor/native/lanes/passive_clock_witness` is the verdict
authority, and each run writes `PASSIVE_CLOCK_INTERVALS_V2.json` so a consumer
divides a rate by active time rather than by wall time. On the collector side a
capture window that observed nothing releases its hooks through a reason that
refuses any lane surrendering a non-empty discard, so an empty window is proved
by every lane's own sealed drain instead of inferred from a missing terminal
frame — the inference a lane that lost its observations would satisfy equally
well.

The evidence-only control path mirrors an engine pause routine rather than one
byte of its result. Clearing `CInGameIdler+0x6C1` alone let the simulation run
while the rest of the pause state stayed set, which is a configuration ordinary
play cannot produce: the pause overlay stayed lit on the date button while the
calendar ticked. The cluster is enumerated from the binary — `+0x6C0` route,
`+0x6C1` latch, `+0x6C2` kind, `+0x6C3` forced, plus two heap-allocating
`std::string` assignments, `+0x800`, `+0x808` and the global at `0x1433723EC` —
by searching `.text` for each offset's exact little-endian `disp32`. That is
exhaustive for a direct access from the object pointer and does not cover an
interior pointer, an overlapping store or a bulk copy, so it is the enumerated
writer set rather than a proved exhaustive one. `Control-Hoi4Game.ps1` mirrors one named routine per
case and refuses states from which no engine routine makes the requested
transition.

The `+0x6C2` overlay hypothesis was tested against a live game and **did not hold as a
sufficient explanation**, and the run did not reproduce the symptom at all. Driven entirely by memory writes on a freshly loaded save, S2 (the old
one-byte poke, kind byte clear) and S4 (the mirror, kind byte set) produced a
date button with no discernible difference -- the S2/S4 comparison is the
observation, and the two steps are separated by a re-pause, so it is a claim
about that sequence rather than a single-variable pair. Neither route
reproduced the reported symptom. The date button tracked the pause latch, which
is consistent with a bare write being enough; the experiment did not invoke
and not a finding. Writing `+0x6C2` remains correct because it is what the
engine's routine writes; it is simply not the explanation. The overlay is
broadcast and the string assignments. Because S2 -- the old one-byte poke, the
configuration reported as stuck -- looked normal here, some condition differing
between the two runs and observed by neither is doing the work, so a
history-dependent contribution from the kind byte is not excluded. **No player
baseline has been run against the mirror, so nothing here validates the fix.**

**A write-only control path cannot reach the engine's full post-state, and the
script reports that rather than presenting a partial transition as a complete
handler's GUI update, and assigns MSVC `std::string`s that allocate in the
target heap; `WriteProcessMemory` performs neither. That remains true of the
FULL state, and whether the OVERLAY needs them is undecided -- the comparison
that appeared to settle it was confounded by the GUI call.

**The acceptance bar -- engine state after our unpause indistinguishable from a
player-unpaused game -- has been run against a live game and FAILED.** The first
attempt left `+0x6C2` clear where the player's route sets it, and the overlay
stayed lit. The `param_2` mismatch behind that has been corrected, which is a
defect in the mirror fixed rather than a validated fix or an explanation of the
overlay; no run has compared the mirror against a player baseline. The bar is
not met.

`CGameState+0x4C0` is a diagnostic and classifies nothing. Reading it as a
clock hold was refuted by observation: a live capture reported it non-zero on
every sample while the hour advanced in every window.

One named defect stands open in that hook machinery. The A31 inline patcher's
`SuspendedOtherThreads::capture`
(`extractor/native/lanes/focus_research_decisions_depth_live/src/a729_focus_item_inline.rs:126`)
means to skip threads that exit between the toolhelp snapshot and the suspend, and tolerates
`ERROR_INVALID_PARAMETER` and `ERROR_INVALID_HANDLE` for that. Windows actually reports an
already-terminated thread as `ERROR_ACCESS_DENIED`, measured by instrumenting `GetLastError`
across five reproductions. The skip therefore never fires for the common case and `capture`
fails closed, refusing the patch, whenever any thread happens to exit during installation —
reproduced in 2 of 40 module runs. It fails safe, so this is an availability defect rather
than a correctness one, but under a live game it makes hook installation intermittently
refuse for a reason unrelated to the patch. Closing it means requesting
`THREAD_QUERY_LIMITED_INFORMATION` in the `OpenThread` call and gating the skip on
`GetExitCodeThread() != STILL_ACTIVE`, which widens which threads a safety-critical patcher
will leave running — the owning lane's call, not a test-lane change.

## Replay layer — playback half

The rendering-laws programme closed its register: every XN01–XN17 non-UI packet and every
XUI01–XUI26 UI packet is proven, or data-blocked waiting on extractor input.

XUI27 replay-owned terminal composition is implemented and has historical green evidence, but
the current source set needs a fresh content-addressed refreeze, and the adversarial
release-review wave has never run. Headless frame-time work is optional performance evidence,
not a product gate.

The 2026-07-19 "replay layer complete" finalisation receipt is **not a valid freeze authority** —
three of its four hashes drifted the same day. It must be reissued after the current fix wave.

Architecture and specifications live under `replay_interface/documents/`.
`portable_replay/native/` holds the four neutral format crates both halves statically link;
the `.battleplan` file is the only runtime interface between them.

## Evidence

Machine-local evidence lives under `C:\hoi4_evidence\`, which carries its own manifest.
Pre-consolidation git history is bundled at `<local-path>`.

The repo keeps only evidence that production code reads or a live test verifies — 42 files,
down from 142. Everything cut is recoverable from git history.

## Documentation policy

`DECISIONS.md` (intent), this page (as built) and `SEAM_CONTRACTS.md` (blocking) are the whole
current-state surface. Per-half deep ledgers sit next to their code.

Specifications that describe the system as built carry a header saying so and naming their own
superseded claims. The repo carries no process or rules documents; operating method is
machine-local (`C:\Users\Example\.claude\CODEX_PROTOCOL.md`).
