# H-A18 exact lifecycle capture automation

## Outcome

Every autonomous engineering prerequisite behind the remaining A18 fixture is
implemented. The remaining action is only to supply or load the game state and
let the owner runner observe it; no human transcription or semantic judgment is
needed.

Frame polling was insufficient for the final gate. Fresh Wave 5 exact-a729
re-export
`20260720T101858Z-w5-a18-dmi2-synchronous-zero-removal-rec-845d4d32` (shard 09,
manifest `9281a49e3f046984a699c3c42fd1ad172361d670aed9586631329255b038cd25`)
exist only inside the daily update; an hourly reader cannot be required to win
that race.

The new paired evidence detours close that instrumentation gap:

- the `0x54beb0` outer detour records coherent before/after instances;
- the nested `0x604d30` detour records the exact half-open erased range while
  its counter is still zero;
- TLS accepts the erase only when its vector-header argument is exactly the
  active container's `+0x28` member;
- the owner is derived mechanically as `container - 0xe58`, then validated by
  the exact `CCountry` vtable, `CCountry+0x08 tag_db_id`, and the container's
  owner back-reference (the shared update function's non-country callers are
  rejected);
- callbacks retain only owners with timed instances, use a bounded queue, and
  poison completeness on every reentrancy, read, lock, or capacity loss.

Evidence mode outputs a debug sidecar, not a new compact domain. Domain 91
remains reserved and unused. Once the fixture passes, production uses the same
paired observer without the sidecar and drains only validated synchronous
zero/removal events into existing domain 67 `DMI2` schema v3 beside the current
instance set.
The collector installs the pair only when domain 67 is the explicitly scheduled
compact lane and the evidence environment gate is set; it seals both detours
before DLL unload and writes exactly one bounded JSON receipt beside the loaded
collector.

## Owner workflow

Run `extractor/native/scripts/Invoke-A18DynamicModifierLifecycleProbe.ps1` with
the release recorder, exact-a729 executable, user directory, collector DLL, the
save path that will be overwritten at each checkpoint, owner tag, timed and
permanent modifier ids, and an output directory.

The runner performs a clean early launch with
`BATTLE_PLAN_ENABLE_A18_DYNMOD_EVIDENCE=1`, watches the chosen save for three distinct
stable contents, and prompts for:

1. `positive_before`, where the timed instance is `N>=2`;
2. `positive_after`, immediately after the next daily decrement (`N-1`);
3. `post_removal`, after the nested hook saw `1 -> 0 -> erase`.

Each checkpoint hashes and parses the save, reads the live exact entry twice,
brackets it with `CGameState+0x468`, and requires exact equality for definition
id, enabled byte, every signed fixed-five value, and remaining days. PID plus
process-creation FILETIME prevents PID-reuse joins. The final verifier also
requires the hook receipt to carry that same process identity, zero loss, a
permanent `-1/null`, positive `N -> N-1`, synchronous zero-before-erase, and
later live/save absence. The recorder manifest must independently bind that PID
to the resolved exact-a729 executable, exact A18-only domain roster, and a
decodable compact stream. The verifier rejects every absolute or inferred
expiry field.

Admission replays every retained daily witness as an ordered vector transform:
decrement every positive counter; require each nested erase to be the exact
current singleton slice in strictly descending ordinal order; then require the
mechanically resulting vector to equal the captured `after` vector. For the
target identity it also requires an unbroken 24-hour chain from checkpoint
`N -> N-1`, through every subsequent decrement to `1`, then the exact erased
`0` row and absence. Pointer-masked entry bytes must decode back to the claimed
day, enabled byte, and value-vector count. Endpoint identity matching or a
skipped intermediate day cannot pass.

## Production state

`PRODUCTION_DMI2_SCHEDULING_ENABLED` and `LIVE_FIXTURE_ACCEPTED` remain false.
The strict zero/removal gate has not been weakened, and no expiry is synthesized.
The exact per-country instance walker, authority-carrying domain-67 DMI2
read/encode/append path, and synchronous zero/removal event queue are wired
behind those latches. The fold accepts a terminal event only at the current
frame hour, against an uninterrupted prior `remaining_days=1` row with equal
enabled/value fields, and only when that identity is absent from the current
set. After a
root-owned accepted fixture and central validation, promotion changes the
latches and pins the lowercase SHA-256 of the verified fixture receipt in
`ACCEPTED_LIVE_FIXTURE_VERIFICATION_SHA256`; changing the booleans without that
nonzero digest cannot activate DMI2. Legacy v1 remains the inactive-path behavior. DMI2 fold admission also
requires the current-hour country-identity roster and carries the resolved owner
tag into every lifecycle transition.

The collector brackets its current-instance walk with two production-event
drains plus active-callback and completed-observation epoch checks. Any daily
update overlapping that walk marks the entire DMI2 frame partial and discards
the staged current/event rows, so a terminal absence cannot be committed ahead
of its synchronous zero evidence.

No Cargo command, build, test, verifier, compiler, or formatter was run by this
subagent.
