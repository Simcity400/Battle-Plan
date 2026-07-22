# X10 direct transition live owner fixture

This is the only live-state requirement left by the X10 static closeout. It no
longer requires a human launch or UI selection: the authenticated acquisition
wrapper performs the clean suspended early launch, scopes the evidence-only
gate to the recorder/game child process, drives the frozen fixture action
profile through foreground `SendInput`, restores `continue_game.json`, and
runs the independent immutable-receipt verifier. The evidence-only activation
does not elevate production authority by itself.

## Preconditions

- exact `hoi4.exe 1.19.2.0.a729`, SHA-256
  `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`;
- all 12 D45/X10 sentinels match before any hook is enabled;
- a fresh process-start nonce and hook-epoch nonce;
- one fixture mod whose authored tokens are recorded in the receipt before the
  game starts;
- a `hoi4.x10.autonomous-owner-fixture-actions.v1` profile bound to the exact
  save SHA-256, product-modset SHA-256, fixture nonce, client size, source
  hashes, action order, owner keys, technology definition index, scope
  identities, and recomputed authored-token hashes;
- `BATTLE_PLAN_D45_X10_OWNER_FIXTURE=1` is set only for the evidence session;
- the production activation constant remains false.

## Required authored cases

The fixture mod must make every case deterministic and uniquely named.

1. Complete one focus.
2. Cross one research completion threshold.
3. Take one ordinary decision.
4. Take one targeted decision.
5. Activate one timed decision.
6. Invoke one outer on-action that invokes a nested on-action.
7. Invoke on-actions from country ROOT, FROM, and FROM.FROM scopes using unique
   durable fixture nonces for grading only.
8. Cause ordinary decision cancel, remove, and timeout effects.
9. Cause targeted decision cancel, remove, and timeout effects.

Normal script/console controls should drive all cases. If the installed script
surface cannot produce a specific cancellation path, the harness may call the
proved wrapper with a real queued decision and real country/target objects. It
must never fabricate a definition, scope, country, or target pointer.

Delayed-event cancellation is not an X10 case: it belongs to D64/A19. Popup
materialization belongs to D75, and decision-panel presentation belongs to D87.

## Machine acceptance

The verifier at `tools/verify_x10_owner_fixture.py` rejects the entire receipt
unless all of the following hold:

- executable, authority jobs/manifests, sentinels, process nonce, hook epoch,
  fixture-mod hash, and authored-token table are authenticated;
- the D45 sequence begins at one, is gap-free, and is ordered exactly as the
  harness action receipt;
- the D45E v3 envelope has one nonzero hook epoch, gap-free drain sequences,
  installed-root mask `0xFFF`, monotonic observed-kind masks, and a clean
  final empty drain whose sequence cursor matches the next expected row;
- the five legacy owner-fixture roots set observed-kind mask `0x01F`; the full
  X10 session sets all 12 bits;
- all 12 kinds are observed from their exact RVAs;
- legacy focus/research/decision kinds have phase `return_observed`,
  `original_returned=true`, and the exact return marker without claiming a
  post-return object re-read;
- all seven X10 kinds have phase `invocation`, `original_returned=false`, and
  `return_bits=0`;
- every decision row has the expected country tag database id;
- every targeted row has a nonzero target opaque id and matches the harness
  target identity;
- every focus/decision/on-action subject hash equals a fresh FNV-1a-64
  calculation over the exact authored token bytes;
- the outer/nested on-action rows preserve invocation order;
- ROOT/FROM/FROM.FROM cases correlate their capture-local scope ids with the
  harness's authenticated stable country/state identity table;
- no raw pointer, pointer-shaped token, address-derived digest, or absolute VA
  appears in the portable receipt;
- collector trailer, recorder authority envelope, typed replay history, and
  replay application expose the same ordered rows and witness mask;
- a clean drain follows the final expected row and diagnostics report no
  active callback, suppressed nested callback, cross-thread sequence, hour
  regression, queue overflow, token decode failure, or sentinel failure.

The clean final drain is not inferred from two adjacent hourly histories. The
generic on-action root can legitimately append rows on each advancing hour.
Instead it is authenticated by the recorder's mandatory final empty drain and
successful hook teardown: any late row, drain error, or uninstall error enters
`collector_control_errors`, prevents the normal replay gate, and is rejected.

The durable fixture nonce exists only to grade scope and cardinality. It must
not become a production replay field or a visually tuned surrogate.

## Autonomous invocation

Use `extractor/native/scripts/Invoke-X10DirectTransitionOwnerFixture.ps1` with
the centrally attested recorder/collector, exact a729 executable, fixture save,
fixture-mod static bundle/bindings, frozen action profile, and a new empty
output directory. The runner refuses late attach and existing game processes.
Its action driver accepts only bounded wait, console command, click, keypress,
and advancing-hour actions; every step is foreground/process/collector bound.
UI input receipts prove dispatch only and explicitly carry
`semantic_effect_confirmed = false`; only the independently captured exact
D45E row can confirm each expected semantic transaction. Advancing-hour steps
select speed one through the numeric-key UI path, read back raw speed index
zero at `CGameState+0x4BC`, and reject any hour overshoot or undershoot.

The accepted output is
`x10_direct_transition_owner_fixture_verification.json`. It contains only
content identities, portable D45E rows, capture-local opaque ids, fixture scope
labels, and hashes. It contains no executable path, profile path, raw pointer,
or absolute VA. No human input or interpretation is part of acceptance.

### Frozen action-profile contract

The action profile is a JSON object with schema
`hoi4.x10.autonomous-owner-fixture-actions.v1`. It must carry the exact
`executable_sha256`, `save_sha256`, `product_modset_sha256`,
`fixture_nonce_sha256`, `expected_game_hour_raw`, expected client width/height,
all eleven `covered_cases`, and a nonempty portable `fixture_sources` roster of
`relative_path` plus SHA-256 rows.

`actions` begins with exactly one `wait_for_loaded_paused_save`. Transaction
steps use only `console_command`, `click`, `keypress`, bounded `wait`, or
`advance_hours`. The last step must be an `advance_hours` action with
`role = "final_drain_settle"` and at least two hours. This gives the hourly D45
collector a post-transaction fold boundary before the recorder performs its
mandatory empty final drain and teardown.

`expected_fixture_rows` is in exact transaction order. Every row carries an
`action_label`, exact kind, owner key, target-presence boolean, and—for
on-actions—a nonce-bound `scope_identity`. Focus and every decision/on-action
row carry the exact authored token and its decimal FNV-1a-64 recomputation.
Research instead carries `authored_token = null`, token hash zero, and the
nonzero stable technology `subject_key`, matching the carrier's deliberately
distinct research identity law. The roster must cover all twelve root kinds;
additional rows distinguish outer/nested and ROOT/FROM/FROM.FROM cases.
