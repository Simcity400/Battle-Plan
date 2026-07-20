# XUI23 — active command abilities

Status: **D88 CARRIER/FOLD/REPLAY/UI WIRED — LIVE A36 ACTIVATION FALSE**

XUI23 closes every replay-owned command-ability seam without evaluating game
triggers or inventing timers, costs, targets, ordering, text, or disabled
reasons. The exact extractor-owned A36 carrier and autonomous same-row source
builder now exist; the remaining input is the authenticated live transition
fixture set, not a rendering approximation or an unsearched capture request.

## Proven authored law

The effective ability definition set comes from `common/abilities`, joined
through the same UI14/UI15/XUI24 VFS receipt used by the production UI catalog.
The vanilla 1.19.2 baseline contains 22 ability definitions in six files. Every
definition has a name key, description key, `army_leader` target type, cost and
duration; two define an explicit 504-hour cooldown and three are indefinite
cancelable abilities (`duration=-1`). Every resolved icon exists in
`interface/abilitylist.gfx`.

`abilitylist.gui` fixes the button container at `(37,11)`, size `36x80`, and
places the active layer, cooldown layer, and button at `(-8,-3)`. The window is
`165x80`, center-down, unclipped, with a 300 ms linear fade. The active and
cooldown backgrounds are separate vertical `33x33` progress bars using
`gfx/FX/progress.lua`. Static `allowed` blocks prove only authored trigger
presence; they cannot be promoted as a runtime availability result.

## Production replay law

`countries.<TAG>.active_abilities` is deliberately separate from the existing
save-side `abilities` object, which contains aces, leader reassignment/capture
cooldowns and officer-corps state—not named active command abilities.

The projected object is one atomic authority envelope: a strict ABA2 carrier
retains the exact executable, exact20-manifest and refresh-decompile hashes,
while its nested ABL1 snapshot retains frame/country/context/source identity,
completeness and rows. No consumer may peel off `rows` or `complete` and discard
that authority. Each exact row carries identity, availability, activation hour,
typed total/remaining timers, exact fixed-point effective command-power cost and
scale, target kind/id, source order, resolved icon, resolved tooltip and exact
disabled reason. `snapshot.complete=true` with `rows=[]` is a proven-empty bar.
All absent, null, legacy-flat, wrong-typed, incomplete, duplicate, out-of-order,
timer-inconsistent, receipt-mismatched, unknown-definition and icon-mismatch
states reject.

`replay_core::PoliticsDetailView` exposes the entire envelope as one
`Absent`/`Unobtained`/`Measured` leaf. `game_ui::active_abilities` revalidates
the envelope before admitting rows and performs no trigger, localization, cost
or timer reconstruction. `not_started` and `indefinite` stay distinct from
numeric `hours`; the command-power value remains an integer with scale `100000`.
The Python snapshot schema enforces the same receipt, identity, type, timer,
fixed-point, completeness and ordering laws.

`load_active_ability_definitions` consumes the authenticated effective VFS,
strictly parses every effective `common/abilities` file, resolves icons against
the effective sprite registry, and rejects parse recoveries, unknown top-level
constructs, duplicate ability identities, invalid authored bounds and missing
sprites. Mod-added abilities therefore enter through the same production path;
there is no frozen vanilla-only runtime table.

## Exact extractor dependency

A36 is frozen separately in
`extractor/documents/dossiers/A36_ACTIVE_ABILITY_INPUTS.json` and listed once in
the extractor-only `RE_REMAINING.md`. It requires a same-tick selected-context
row set plus affected-division identities for the existing on-counter
`active_ability_kinds` seam. Static definitions, save commander cooldowns and
RenderDoc draws are explicitly forbidden substitutes.

The retained XN16P receipt binds 214 RDCs totaling 69,298,844,274 bytes. Those
captures remain useful for pixels and authored-layer corroboration, but a draw
stream cannot supply the complete identity/availability/target/timer/text row.
No additional capture is requested for the replay-owned implementation.

The extractor now reserves compact domain 88 and wraps the existing ABL1 row
codec in the receipt-bound ABA2 production envelope. Recorder authority and
world fold decode the envelope, reject unavailable/torn/receipt-mismatched
payloads, and project the full carrier plus snapshot into
`countries.<TAG>.active_abilities`. Replay and UI also preserve and validate the
exact `affected_division_ids` set used by the counter-overlay ability seam.
ABA2 rejects pointer-shaped identity strings and derives its source-record id
from the exact hour, country, context kind, and portable context identity.

Scheduling remains false. Exact20/Ghidra evidence proves that `0x141D049C0`
joins the callback widget to `controller+0x70` row order and then indexes the
same `controller+0x718` ability definition before returning the final tooltip.
The exact observer now intercepts the unique append returning at `0x141D06057`,
copies the same-row temporary accumulator before destruction begins at
`0x141D063A3`, and transactionally joins it to the refresh row owner/source
vectors before pointer erasure. It also records the exact effective-cost call
returning at `0x141D04E38`, the chosen icon resolver input returning at
`0x141D08BA6`, and the intrinsic definition token. The source constructor
accepts no caller-built ability frame: it derives every ABL1 field and joins
portable context/division identities through the same-epoch exact root catalog.
It does not parse the final tooltip. Scheduling remains false because no
retained live capture supplies the required ordinary, cooldown, multi-leader
field-marshal and measured-empty game-state matrix.

The recorder-fold, replay-core, and game-ui regressions independently consume
`canonical_active_abilities_envelope.json` (SHA-256
`9b05c17bf792fc01a7dfcb34088fbd45e03dc1105b36eb258240cc3eedea7efd`) and each
rejects an authority mutation. This freezes one production shape without adding
cross-workspace test dependencies.

## Mechanical freeze

`build_xui23.py` deterministically rebuilds the static catalog and retained
source census. `verify_xui23.py` hashes every production source and dossier,
checks the focused Python/Rust test contracts statically, and writes the
verification and manifest with
`PASS_STATIC_CONTRACT_CENTRAL_RUST_VALIDATION_REQUIRED`. It executes no test
runner or compiler. Rust validation is deliberately external: the primary
agent runs it once via
`Invoke-CentralRustValidation.ps1 -Mode Final -Workspace All`, so this packet
verifier cannot start a competing Cargo target or compile. Both freeze passes
must be byte-identical before promotion.

## Incidental findings

- `extra_suplies` is the shipped script identifier (misspelling preserved),
  while its texture path is `ability_extra_supplies.dds`.
- The shipped GUI comment still names `basic_abilities.txt`, but the effective
  vanilla definition is `generic_leader_abilities.txt`; the comment is not used
  for resolution.
- Save-side `leader_modifier_enable_date` is a reassignment/capture gate and
  must not be displayed as an ability cooldown.
- The original static-catalog builder serialized eight shipped-source paths as
  machine-local `F:/...` paths. The frozen builder now records
  `root=installed_game` plus canonical game-relative paths; all eight file
  hashes and sizes validate without retaining a workstation-specific path.

## Ghidra provenance

| Function address | Job ID | Artifact path | Manifest hash |
|---|---|---|---|
| `0x141D072F0` | `20260720T020812Z-w3-a36-ability-list-feb98226` | `evidence/func_141d072f0.decompile.c` | `12768dfa9a8b47ceda4feae9cdc45b76811f602ecd557008aa936e95adf2db45` |
| `0x141D049C0` | `20260720T020812Z-w3-a36-ability-list-feb98226` | `evidence/func_141d049c0.decompile.c` | `12768dfa9a8b47ceda4feae9cdc45b76811f602ecd557008aa936e95adf2db45` |
| `0x140FF44E0` | `20260720T063948Z-w4-a36-definition-identity-icon-roots-f4346705` | `evidence/func_140ff44e0.decompile.c` | `e64f8324769ec88de5bc2c09364097bc57336304e46fe620f3e40469dee4c5e1` |
| `0x140FF5B20` | `20260720T070828Z-w4-a36-explicit-icon-override-0b0e8bcb` | `evidence/func_140ff5b20.decompile.c` | `514d188b2dd8f2cbb8f384c35cd10d1e97cfe34527ce3ecb71bdbdcc8cf2c493` |

These are retained exact20 artifacts re-read after the mandatory current-broker
status gate; no analysis job was launched. `PROGRAM_STATUS`, unrelated packets
and adversarial review surfaces were not edited.
