# LAND foreign-unwind fixtures

These diagnostic-only, independently authored native frames exercise production
LAND callbacks with the shared [probe DLL and client](README.md). They run outside
the game and recorder. No Rust catcher, exception translation, copied emitter or
non-unwinding callback cast substitutes for actual foreign dispatch.

The shared Hook provides atomic publication and `new_indirect_call` owner safety.
LAND retains escaped flights. Cases and raise pointers remain inside the owning
loaded `Probe` lifetime.

## Covered production paths

- `production_land_detour_foreign_unwind` installs the real damage entry shim
  through `Hook::new`. An authored eight-slot native caller forwards four register
  and four stack arguments through the production shim, Rust body, pre observation,
  relocated original and post path. Native mutation precedes the real C++ throw
  or SEH raise. Foreign cleanup must retain ACTIVE while closing the observation,
  refusing the incomplete interval and retaining storage.
- `production_verdict_indirect_foreign_unwind` installs the real leader resolver
  stub at an authored six-byte indirect CALL. The relay returns to the padding after the five-byte CALL
  and continues after the six-byte CALL on normal return. Native resolver arguments, leader
  handle, continuation, nonvolatiles and registered frames are witnessed. Escaped
  unwind must leave the ASM-owned flight sticky, stop observation, refuse drain
  across load, retain the patch/log and fail restoration. A subsequent normal
  native call retires only its own flight. This is the production retention policy.
- `production_return_call_foreign_unwind` patches an authored five-byte CALL
  with the real HQ return wrapper. Its authenticated pre read establishes an
  unsettled source head before native reserve credit and the foreign raise.
  Cleanup must retain ACTIVE, refuse drain, keep the source unavailable across a
  gap until a load clears it, and retain charged storage. A load cannot prove that
  Windows has finished traversing the callback or permit unload.

Typed assignments require the actual Rust bodies and original entry declarations
to remain `system-unwind`; the former `system` types cannot satisfy them. The
shared DLL handler result proves the matching C++ destructor or SEH `__finally`
ran. Its common assembly witness checks all eight general nonvolatiles, XMM6..15
and actual RSP at native handler continuation. Each case also checks original
call/return counts, skipped continuation, native mutation, family counters,
refusal state, storage ownership and full-sentinel restoration. The verdict path
also checks that later normal work cannot retire an earlier escaped flight.

Each fixture's test-only reset occurs only after its authored single-thread native
catch and any later normal driver have returned with their complete witnesses.
No native return or observer frame remains in that isolated trace. Production has
no reset or completion API. This does not establish concurrent detach safety or
permit unloading a collector with an escaped flight.

These paths cover damage entry, verdict indirect call and accepted HQ return.
Other supplemental callbacks, native game handler scopes, asynchronous faults at
arbitrary Rust instructions, concurrent detach, actual unwind metadata/artifacts,
fresh saves, smoke and live cost require separate validation. The existing direct
Rust-panic policy regression is also separate; it is not foreign-unwind evidence.

## Build and run

Run from the repository root in an x64 MSVC developer environment. Keep
all DLLs, objects, logs, metadata and debugger dumps outside Git.

```powershell
$proofOutput = Join-Path $env:TEMP 'battleplan-land-foreign-proof'
New-Item -ItemType Directory -Force -Path $proofOutput | Out-Null
cl.exe /nologo /LD /O2 /EHa /W4 /WX tools/tap-foreign-unwind/probe.cpp "/Fo:$proofOutput/probe.obj" "/Fe:$proofOutput/tap_foreign_probe.dll" /link "/IMPLIB:$proofOutput/tap_foreign_probe.lib"
if ($LASTEXITCODE -ne 0) { throw 'Native probe build failed' }
$env:BATTLEPLAN_TAP_UNWIND_PROBE = "$proofOutput/tap_foreign_probe.dll"
$landProofs = @(
    @{ Crate = 'collector-equipment-destruction'; Filter = 'equipment_destruction::tests::foreign_unwind::production_land_detour_foreign_unwind' },
    @{ Crate = 'collector-land-events'; Filter = 'land_events::foreign_unwind::production_verdict_indirect_foreign_unwind' },
    @{ Crate = 'collector-equipment-destruction'; Filter = 'equipment_destruction::tests::foreign_unwind::production_return_call_foreign_unwind' }
)
foreach ($proof in $landProofs) {
    cargo test -p $proof.Crate --features tap-foreign-probe $proof.Filter -- --exact --ignored --nocapture --test-threads=1
    if ($LASTEXITCODE -ne 0) { throw "LAND foreign proof failed: $($proof.Filter)" }
}
```

Require exactly one selected test per command, exit zero and all nine case `PASS`
lines (normal/C++/SEH for each path). A skipped or absent test is unverified.
Run the existing four-check [generic Tap example](README.md#build-and-run)
separately. Repeat at the final collector profile if it differs from the test
profile; successful diagnostic builds alone do not certify a release artifact.

Record the exact merged head, compiler versions/profile, commands, exit codes,
output, executable/DLL hashes and unwind tables. Under a Windows debugger, capture
disassembly and registered runtime entries for the printed native site, production
stub/body and generated ranges while the DLL raise is active. Verify each actual
handler path and continuation; do not replace it with virtual-unwind reconstruction
or an earlier artifact.
