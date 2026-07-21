# X10 direct transition live owner fixture

This is the only external-state requirement left by the X10 static closeout.
It requires a human to start/authorize one exact-a729 harness session. Grading
after launch is mechanical; the evidence-only activation does not elevate
production authority.

## Preconditions

- exact `hoi4.exe 1.19.2.0.a729`, SHA-256
  `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`;
- all 12 D45/X10 sentinels match before any hook is enabled;
- a fresh process-start nonce and hook-epoch nonce;
- one fixture mod whose authored tokens are recorded in the receipt before the
  game starts;
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

The verifier must reject the entire receipt unless all of the following hold:

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

The durable fixture nonce exists only to grade scope and cardinality. It must
not become a production replay field or a visually tuned surrogate.

## Human input required

The required human action is limited to launching/authorizing the exact-a729
fixture session and, if automation cannot select the targeted UI row, selecting
the named fixture target. No human interpretation of offsets, tokens, order,
scope semantics, or pass/fail is permitted.
