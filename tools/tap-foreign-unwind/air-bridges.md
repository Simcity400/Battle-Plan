# Air bridge foreign-exception diagnostic

This diagnostic uses the shared, independently authored
[native probe DLL](README.md). Its two ignored collector tests install the actual
production hooks over authored native frames and call the actual Air assembly
bridges. No game, recorder, copied emitter or Rust exception catcher is involved.
The `tap-foreign-probe` feature excludes the diagnostic from ordinary builds.

The air-loss case patches a five-byte CALL with `Hook::new_call` and invokes
`air_loss_entry`. The building case uses `Hook::new` to relocate the same
five-byte home-space store as the existing synthetic native fixture, then invokes
`building_damage_entry` and its real trampoline. Both reuse the existing
heap-owned `World` inputs and production guarded reads. The first normal call
must complete a real pending claim and publish an exact native event; a second
normal call leaves owned event storage in the ledger before the exception.

Each C++ and SEH case then raises from the authored native original, **after the
production pre callback has returned and before post**. The shared native handler
must certify the C++ destructor or SEH finally and expected handler continuation.
The production post callback and the caller's normal continuation must be skipped.
There is no Rust callback frame or Rust cleanup guard on this throwing path.

Skipping post intentionally leaves one active observer and its pending claim.
The tests require stranded/poisoned status, named drain refusal, lost-observation
accounting and unchanged claim/allocation across subsequent hourly boundaries.
A later normal hooked call must still return from the native original while
capture stays refused; it cannot release the stranded claim. Disabling restores
all sixteen sentinel bytes. The diagnostic keeps both executable ranges and
their bytes owned, then calls the restored native path and verifies its exact
return PC, arguments and result without another observer entry.

The final ledger reset is test teardown after every synthetic call and native
handler has returned and the hook has been removed. It is **not production
recovery**. Production `runtime::wait_quiet` continues to refuse quiescence while
the active observer is stranded and retains its owner. These tests do not invoke
that blocking runtime loop or certify DLL unload after a foreign exception.

The shared diagnostic runner also witnesses all eight general nonvolatiles,
XMM6..15 and RSP at the native handler's actual continuation. The Air fixtures
record their native-original return PC and exact bridge/caller RSP delta separately; the
building fixture changes its saved nonvolatiles before raising.

This proves only deliberate native-original C++/SEH exceptions through the two
authored callers. It does not certify exceptions inside the production pre/post
callbacks, asynchronous faults at arbitrary Rust instructions, game handler
scopes, every native patched site, live capture cost or actual detach/unload.
The synthetic building caller is intentionally classified as an unknown game
caller; native caller allow-list acceptance remains a separate live check.

## Run

Set
`BATTLEPLAN_TAP_UNWIND_PROBE` to the absolute path of that separately built DLL. Use the
same x64 MSVC `/EHa` build and chosen scratch directory as the shared guide.

From the repository root, run these focused diagnostics; do not replace them
with the virtual-unwind tests:

```powershell
$proofOutput = Join-Path $env:TEMP 'battleplan-tap-foreign-probe'
$env:BATTLEPLAN_TAP_UNWIND_PROBE = "$proofOutput/tap_foreign_probe.dll"
cargo test -p collector-observers --lib --features tap-foreign-probe foreign_unwind_retains_stranded_observation_and_restores_call -- --ignored --nocapture --test-threads=1
```

Require both tests to pass and four `PASS` case lines: air-loss C++/SEH and
building-bombing C++/SEH. A crash, missing cleanup/handler, register/RSP mismatch,
unexpected capture recovery, stale claim loss or failed restoration is a failure.
Record the exact integrated source, compiler versions/profile, executable and DLL
hashes, commands, output and exit status. Repeat in the final collector profile
if it differs. Inspect actual PE unwind entries and bridge/caller addresses in
the final artifact; keep binaries, logs and debugger evidence outside Git.
