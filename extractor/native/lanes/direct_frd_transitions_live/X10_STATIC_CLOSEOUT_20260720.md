# X10 direct transition static closeout (exact a729)

Date: 2026-07-20

This closes the static reverse-engineering and production-path implementation
for X10. It does **not** activate the hooks. One authenticated live owner
fixture remains mandatory because retained saves and hourly captures cannot
prove that transient invocations occurred, or bind the generic on-action scope
to portable ROOT/FROM identities.

## Binary authority

- binary: `hoi4.exe 1.19.2.0.a729`
- image base: `0x140000000`
- SHA-256:
  `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`
- broker topology at export: 20 ready read-only shards

The existing five post-return roots remain bound by job
`20260720T020813Z-w3-focus-direct-hooks-d987b89a`, manifest
`a9855541571204160bc4ac78dcf6f1dbdfa112496234e84540406a71c37d2fb3`.

The generic on-action dispatcher is bound by job
`20260720T140344Z-x10-generic-on-action-and-bridges-root-o-a2a0ea16`,
manifest
`8012d37ac03a47187bc2744e509acac0dbec1e6a7a91ffd20d45d58903e0e3da`.
Its `0x140A6AAE0` decompile artifact SHA-256 is
`004808d8f22c0ac1fdbc6c04c48c20a1f5490bb4671f9340f92fa1c902da86c7`.

The six decision terminal-effect wrappers are bound by job
`20260720T135537Z-x10-direct-and-decision-terminal-mutator-e7b8e477`,
manifest
`213944e845d02896efd26314003a7880384e9d1eddf4a2b799d4449632da8227`.

The final cluster-exhaustion export is job
`20260720T214321Z-w5-x10-on-action-cluster-and-bridges-fin-3733bf4e`,
manifest
`d94eae0f339ee53f6ce69ecc921925519f48138da756ebef353304e31a8d9603`.
It covers 168 functions and 672 decompile/disassembly/p-code/xref artifacts on
`shard-11` under the same exact binary authority.

| Kind | RVA | Observation | Entry sentinel (16 bytes) | Decompile SHA-256 |
|---|---:|---|---|---|
| `on_action_invoked` | `0x00A6AAE0` | entry | `48 89 5c 24 08 48 89 74 24 10 48 89 7c 24 18 55` | `004808d8f22c0ac1fdbc6c04c48c20a1f5490bb4671f9340f92fa1c902da86c7` |
| `decision_cancel_effect_invoked` | `0x0071B350` | entry | `48 89 5c 24 10 48 89 74 24 18 57 48 81 ec f0 00` | `1464ed276b4695d682f7973dd240b3b3aa495dba401b3f1a40f0e8bad5e6f6d4` |
| `decision_remove_effect_invoked` | `0x0071B590` | entry | `48 89 5c 24 10 48 89 74 24 18 57 48 81 ec f0 00` | `7d52a90d0624c4c94d251fd6cd8030b5ee2b71867c6c12e452a6ece62816c0ff` |
| `decision_timeout_effect_invoked` | `0x0071C490` | entry | `48 89 5c 24 10 48 89 74 24 18 57 48 81 ec f0 00` | `bf036ff487f737fbe545bb119a2d27ac9c4a1337d93988ea16c5aa81f278f427` |
| `targeted_decision_cancel_effect_invoked` | `0x0071B7D0` | entry | `48 89 5c 24 10 48 89 74 24 18 48 89 7c 24 20 55` | `2dd3d0693a4702213e2d2a211e202248dbd91888fa38f3599ee7193d965a2358` |
| `targeted_decision_remove_effect_invoked` | `0x0071BE10` | entry | `48 89 5c 24 10 48 89 74 24 18 48 89 7c 24 20 55` | `c20013084441578e5b8e17d7b270d7d512b7e931cb7f717a616a662e3eb414e0` |
| `targeted_decision_timeout_effect_invoked` | `0x0071C130` | entry | `48 89 5c 24 18 48 89 74 24 20 55 57 41 55 41 56` | `bcd2ec00f09cc1b16ffb18a2b444a8bb41d2ab59eab287f6400c2d6789cce543` |

`0x140A6AAE0` has the proved ABI
`(manager, on_action_definition, event_scope, ignore_running_check)`. It checks
the running-game state, executes direct and random event lists, calls the
weighted selector, and executes the definition's effect. The exact authored
on-action token is the MSVC string at definition `+0x20`.

The decision wrappers receive the decision definition, country, and optional
target. They invoke the exact cancel (`+0x4B8`), remove (`+0x460`), or timeout
(`+0xC30`) effect slot. The authored decision token is the MSVC string at
definition `+0x120`; the stable owner key is the country tag database id at
country `+0x08`.

## Rejected alternate roots

- `0x140723060` is a broad hourly decision lifecycle loop. It is retained only
  as a correlation boundary and is not an exact transient-event producer.
- `0x14072A320`, `0x140729EC0`, and `0x14072A000` are timed/targeted lifecycle
  loops, not replacements for the narrow terminal wrappers.
- `0x140A6B100` is parser/registration code.
- `0x140A670D0` is parser/debug code.
- `0x140A684B0` initializes the on-action catalog.
- `0x140A6A350` is the weighted-random selector called by the dispatcher.
- `0x14072B1D0` is decision random-count bookkeeping.
- `0x140207710` is a focus completion notification bridge, not the generic
  on-action dispatcher.

The final exhaustive export reconfirmed this classification. In particular,
`0x140A6AAE0` calls event executor `0x140A026A0` for direct and random event
leaves, while the authored on-action effect is invoked polymorphically through
the effect object's virtual slot. There is no single stable universal indirect
effect leaf. Hooking those downstream leaves in addition to the dispatcher
would double-count some invocations, so the generic dispatcher invocation is
the correct universal observation boundary. The scope validator at
`0x14116B5F0` is a validator, not a producer.

Final cluster decompile artifact hashes:

| Address | Classification | SHA-256 |
|---|---|---|
| `0x140A670D0` | parser/debug catalog | `97b5e0d0215d6ea246dd10c60c8f154992201a26a68c4570350c5ed445da5811` |
| `0x140A684B0` | registered catalog initializer | `711569b5e41b89d7a5f5da6b51369d8867987c3d35bc766259f77c9ed36a5ec8` |
| `0x140A6A350` | weighted selector | `da07478a2d1d5115c8740b0f69ae946fa4c4d3c365dc23e7c72b6e6bce73d484` |
| `0x140A6AAE0` | universal dispatcher | `004808d8f22c0ac1fdbc6c04c48c20a1f5490bb4671f9340f92fa1c902da86c7` |
| `0x140A6B100` | event/random-event list parser | `44dac2eef10f69b7761139c706a88096e7bb1094ec1830e3e96c9d26710d87b6` |
| `0x140207710` | focus notification/localization bridge | `865c2781138d439a02b6f7771eb7e2b407c04ea25238694536430b621683fa83` |
| `0x14072B1D0` | random-count bookkeeping | `07a7576e51ddc5130b4b21a2c41c9ceefd27e7af46d0cdb407876d8c67727f73` |
| `0x140A026A0` | downstream direct/random event executor | `7567ea6a4cb945dda034af74976c90a3d4c93d092ffe37c778bfa8a6d963413b` |
| `0x14116B5F0` | event-scope validator | `f6e0c05b0c23fbd5cc981c3bf200df738dfa15b0ea5fbe116bb119cea0ca435b` |

## Production-path implementation

X10 reuses D45E; it does not allocate another compact domain and does not
duplicate delayed-event (D64), popup/materialization (D75), decision-panel
(D87), or proof-only timeline (D249) semantics.

D45E trailer version 3 has a 56-byte authority header and 120-byte
pointer-free rows. The header binds the hook epoch, gap-free drain sequence,
installed 12-root mask, cumulative observed-kind witness mask, next-row
sequence cursor, producer-completeness bit, and evidence/production mode. The
row carries:

- stable owner key (`u64`);
- FNV-1a-64 of the validated exact authored subject token (`u64`);
- the stable `CTechnology+0x08` definition index for research (`u64`);
- an explicit `invocation`/`return_observed` phase in portable JSON;
- exact entry scalars plus an honest return marker for legacy roots.

The collector, recorder fold, portable JSON projection, typed replay decoder,
root validation, sequence validation, phase validation, target-presence
validation, epoch/drain continuity, witnessed-kind monotonicity, and
fail-closed malformed-history behavior are wired. Invocation rows
are emitted before the original call because the invocation itself is the
transient fact. The five legacy roots are emitted after the original returns,
but do not claim that mutable engine objects were safely re-read after return.
Teardown is two-stage: all roots are first quiesced, in-flight callbacks are
counted, and removal/state reset is refused until the count reaches zero.

No address, pointer-derived hash, absolute VA, or D3D identity crosses the
carrier. Scope/definition/target pointers receive capture-local monotonic opaque
ids. Authored-token hashes are calculated from validated token bytes, never
from pointer bits.

## Exhaustion result and remaining live proof

The installed exact-a729 data set contains 128 event files, 88 decision files,
and 17 on-action files. These scripts prove that delayed events, decision
lifecycle changes, nested on-actions, and transient ROOT/FROM/FROM.FROM scopes
are real authored cases. They cannot prove the hook's live ABI, invocation
cardinality, or portable scope join because those facts are absent from saves,
hourly polls, RenderDoc captures, and prior harness captures.

The evidence-only fixture can be activated without changing production
authority by setting `BATTLE_PLAN_D45_X10_OWNER_FIXTURE=1`. Its portable D45E envelope
retains the hook epoch, drain completeness, and cumulative per-kind witness
mask; in-process diagnostics retain per-kind counts. The legacy five-root
owner fixture is complete only after all low five witness bits are present.

The sole remaining X10 dependency is the machine-graded live owner fixture in
`X10_DIRECT_TRANSITION_OWNER_FIXTURE_20260720.md` and its JSON contract.
Production activation must remain false until that fixture passes. No semantic
labeling or subjective human judgment is requested.
