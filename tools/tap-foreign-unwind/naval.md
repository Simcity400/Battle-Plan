# Naval foreign-unwind fixtures

The feature-gated `naval_original_cpp_seh_preserves_cleanup_storage_and_restores_hooks`
uses the existing [native probe and shared register witness](README.md). It adds
no DLL loader, emitter or exception framework. Independently authored native
caller/original frames install the actual `constructor_entry`, `fill` and `merge`
bridges with the production Hook entry and 16/13-byte CALL APIs. Native original
code calls the real C++ throw or noncontinuable SEH raise directly, after the
diagnostic Rust helper returns. Native handlers, destructor/finally counts, all
eight general nonvolatiles, XMM6..15 and actual handler-continuation RSP remain
the shared probe's assertions.

Each path verifies normal arguments/return, original RSP's exact depth from that
invocation's measured caller RSP, alignment and registered return PC. C++ and SEH
have different outer native handler frames; their absolute stack addresses need
not agree. It also verifies skipped continuation, one actual `Call` cleanup, zero
ACTIVE, poison/unwind refusal, folded event bytes and the drain's storage
ownership. While native original code is active, a diagnostic-only flag change
isolates ACTIVE's release veto; it restores the flags before native dispatch.
After dispatch, the test-owned installed-site flag independently vetoes release.
A later normal call must preserve the poisoned observation and queued storage.
Disable restores the entire sentinel, retains the Hook's executable allocations
until its drop, and demonstrates native passthrough at the five-byte return point and the original
RSP. Storage release occurs only after native handlers have returned, ACTIVE is
zero and the single owned site is restored. No stranded counter is reset.

Naval's native original runs inside the unwind-capable Rust `Call` guard. This
fixture requires its cleanup on C++ and SEH; a stranded guard is a failure, not
accepted recovery. The constructor case authenticates the real shim return PC
as the load-factory path, which deliberately records no birth. The result cases
use owned synthetic game operands and confirm an actual emitted new/merge row
before retaining the next unpublished row. No fresh native result is invented.

The existing `native_foreign_unwind_probe` and its independently authored
`crates/collector-runtime/tests/naval_unwind_probe.cpp` remain unchanged. That narrower
C++ proof covers the constructor's unknown-caller path and both result CALLs.
Keep both checks: the shared proof adds SEH and complete continuation witnesses;
it does not replace those assertions.

This isolated single-hook diagnostic does not certify whole-family installation,
concurrent detach, persistent/transient constructor birth reads, game handler
scopes, asynchronous faults at arbitrary Rust instructions, or final artifact
unwind metadata. It introduces no production recovery/reset. Fresh-save, smoke,
live capture, cost and actual unload require separate validation.

## Build and run

Run from the repository root in an x64 MSVC developer environment. Keep
every artifact and log outside Git.

```powershell
$proofOutput = Join-Path $env:TEMP 'battleplan-naval-foreign-proof'
New-Item -ItemType Directory -Force -Path $proofOutput | Out-Null
cl.exe /nologo /LD /O2 /EHa /W4 /WX tools/tap-foreign-unwind/probe.cpp "/Fo:$proofOutput/probe.obj" "/Fe:$proofOutput/tap_foreign_probe.dll" /link "/IMPLIB:$proofOutput/tap_foreign_probe.lib"
if ($LASTEXITCODE -ne 0) { throw 'Shared native probe build failed' }
cl.exe /nologo /LD /O2 /EHa /W4 /WX crates/collector-runtime/tests/naval_unwind_probe.cpp "/Fo:$proofOutput/naval.obj" "/Fe:$proofOutput/naval_unwind_probe.dll" /link "/IMPLIB:$proofOutput/naval_unwind_probe.lib"
if ($LASTEXITCODE -ne 0) { throw 'Existing Naval C++ probe build failed' }
$env:BATTLEPLAN_TAP_UNWIND_PROBE = "$proofOutput/tap_foreign_probe.dll"
$env:BATTLEPLAN_NAVAL_UNWIND_PROBE = "$proofOutput/naval_unwind_probe.dll"
$navalProofFilters = @(
    'naval_result_sources::tests::native_foreign_unwind_probe',
    'naval_result_sources::tests::foreign_unwind::naval_original_cpp_seh_preserves_cleanup_storage_and_restores_hooks'
)
foreach ($proofFilter in $navalProofFilters) {
    cargo test -p collector-naval-results --features tap-foreign-probe $proofFilter -- --exact --ignored --nocapture --test-threads=1
    if ($LASTEXITCODE -ne 0) { throw "Naval foreign proof failed: $proofFilter" }
}
```

Require exactly one selected test per command, both passing test results, exit
zero, and six new `Naval <site> <C++|SEH> PASS` lines (new result, merge and load
constructor). Normal checks run inside each case. A missing/ignored selection,
crash, missing cleanup/handler, stranded counter, register/RSP/PC mismatch,
unexpected observation recovery or failed restoration is a failure.

Record exact source, Rust/MSVC versions/profile, commands, exit status, output
and executable/DLL hashes. Repeat with the final collector profile if it differs.
Inspect the actual PE unwind tables and, under a Windows debugger at each native
raise, the printed caller/site/bridge/original PCs and Hook-owned ranges. Keep
disassembly, registered runtime entries and handler evidence in the chosen proof
folder. Earlier artifacts or virtual-unwind reconstruction do not replace this
evidence.
