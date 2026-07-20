# H-A18 exact lifecycle capture automation

## Outcome

Every autonomous engineering prerequisite behind the remaining A18 fixture is
implemented. The remaining action is only to supply or load the game state and
let the owner runner observe it; no human transcription or semantic judgment is
needed.

Frame polling was insufficient for the final gate. Fresh exact-a729 job
`20260720T052710Z-w4-a18-zero-removal-hook-contract-5d917e1b` (shard 11,
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
  `CCountry+0x08 tag_db_id`;
- callbacks retain only owners with timed instances, use a bounded queue, and
  poison completeness on every reentrancy, read, lock, or capacity loss.

The output is a debug evidence sidecar, not a new compact domain. Domain 91
remains reserved and unused. Once the fixture passes, production continues to
use existing domain 67 with the already implemented flagged `DMI2` payload.
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

## Production state

`PRODUCTION_DMI2_SCHEDULING_ENABLED` and `LIVE_FIXTURE_ACCEPTED` remain false.
The strict zero/removal gate has not been weakened, and no expiry is synthesized.
The exact per-country instance walker and authority-carrying domain-67 DMI2
read/encode/append path are already wired behind those two latches. After a
root-owned accepted fixture and central validation, promotion only changes the
latches; legacy v1 remains the inactive-path behavior. DMI2 fold admission also
requires the current-hour country-identity roster and carries the resolved owner
tag into every lifecycle transition.

No Cargo command, build, test, verifier, compiler, or formatter was run by this
subagent.
