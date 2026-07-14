# W12G auto-attach correctness fix

## Mission and verdict

Close the seven reviewed auto-attach/heartbeat findings without changing the
collector or WMI COM ABI. **Offline verdict: PASS. Live verdict: NOT RUN** (the
sandbox cannot drive the game, inject a DLL, or validate the WMI provider).

## Changes and correctness argument

- `auto_attach/singleton.rs`: retained SID/session-scoped Windows mutex rejects a
  concurrent supervisor before recovery/enumeration.
- `auto_attach/coordinator.rs`, `journal.rs`: exact committed intent is
  `sync_data` durable before spawn; uncertain recovery blocks construction;
  pre-spawn identity/window re-probe rejects PID reuse.
- `auto_attach/sources.rs`, `reducer.rs`: Toolhelp returns a completeness witness;
  only a complete enumeration positively lacking a resolved PID can infer exit.
- `auto_attach/staging.rs`, `proof.rs`, `auto_attach.rs`, `main.rs`: the
  coordinator-owned token/session/challenge feed the actual control file, H4CR
  pipe, and H4CT proof. Connection proves residency; a complete first record
  proves production; terminal success also requires both proofs and a retained
  nonempty replay.
- `lib.rs`: exposes the prerequisite `control_transport` module.

This lane changes supervisor lifecycle/proof semantics only; it does not change
compact record contents or world folding.

## Gates

- Baseline library: 261 passed / 1 ignored.
- Final library: 288 passed / 1 ignored (**+27 passing**).
- Final binary: 199 passed / 2 ignored.
- `cargo build -p recorder_host`: green.
- Targeted rustfmt check on changed auto-attach sources and `lib.rs`: green.
- `git diff --check`: green.

## Refutation and residuals

Prime suspects: mutex lifetime/namespace; crash windows; PID reuse between stage
and hook; Toolhelp truncation; token mismatch/leak; stdout proof ordering; torn
first records; child-exit races; and false artifact success. The required fresh
read-only Codex review was attempted, but the sandbox denied websocket access
with OS error 10013, so no independent verdict is claimed.

The implementer's self-refutation found and fixed five issues before commit:

1. terminal `Failed`/`Complete` history with an earlier `Injecting` record was
   not re-seeded on restart, allowing same-FILETIME reinjection;
2. proof-thread setup failure or a live `WorkerStatus::Failed` could orphan the
   spawned child instead of stopping it;
3. extra worker args could override coordinator authority and the default worker
   was not forced onto the compact proof format;
4. the staged DLL needed a second pre-hook hash validation; and
5. first-record proof rejection happened before raw-stream persistence, dropping
   the offending forensic chunk.

All five are covered by the final source/tests. Independent adversarial review
remains an orchestrator gate before merge.

Residual live-only gates: real WMI delivery/recovery, real per-attempt pipe
connection, exact one-injection observation in the game, and replay retention under a
real session. Process-lifetime capture/tray graceful stop remain separate product
work and are not claimed here.
