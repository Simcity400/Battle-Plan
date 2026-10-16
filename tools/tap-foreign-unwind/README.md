# Tap foreign-exception oracle

An independently authored fixture for actual C++ and noncontinuable SEH exceptions through the production Tap, outside the game. The [collector example](../../crates/collector/examples/tap_foreign_unwind.rs) includes production [hook.rs](../../crates/collector-hooks/src/hook.rs); [probe.cpp](probe.cpp) supplies native throw/catch and `RaiseException`/`__finally`/`__except` paths.

Building requires the production Tap implementation with a `system-unwind` observer API. The observer uses that ABI directly; do not cast a non-unwinding callback to make it compile. The `tap-foreign-probe` feature gates the example so ordinary builds do not require its native DLL.

## Layout and contract

[runtime.rs](runtime.rs) is the shared diagnostic client used by this example and LAND/AIR diagnostics. `Probe::load` loads `BATTLE_PLAN_TAP_UNWIND_PROBE`, defaulting to `tap_foreign_probe.dll`. Entry and raise functions use typed `system-unwind` boundaries. Its assembly wrapper records the native handler result, nonvolatile registers and RSP at the actual caller continuation.

The normal case returns through the copied instruction span. C++ and SEH cases unwind from the Rust observer through the generated caller to the native handler. Assertions require one Rust `Drop`, zero active observers, no post-throw continuation, the expected native return PC after the five-byte CALL and RSP, all eight general nonvolatiles and XMM6..15, and native destructor/finally cleanup. Normal flights finish only after the copied instructions. C++ and SEH escapes retain their bridge flights even when Rust cleanup ran; restoration must refuse and the generated unwind lookup must remain registered. The process exits with that synthetic native site and Tap retained.

This exercises exception dispatch, not just `RtlVirtualUnwind` reconstruction. It neither catches Rust panics nor translates foreign exceptions. A crash, unexpected handler, missing cleanup or failed restoration fails the run. The explicit foreign SEH call does not certify asynchronous faults at arbitrary Rust instructions. Game handler scopes, reached patch sites, collector lifetime and gameplay need separate validation in the [collector workflow](../../crates/collector/README.md).

## Build and run

Run from the repository root in an x64 MSVC developer environment with Rust and LLVM's inspection tools available. Keep native artifacts, logs and debugger dumps in a chosen scratch directory outside Git:

```powershell
$probeOutput = Join-Path $env:TEMP 'battle-plan-tap-foreign-probe'
New-Item -ItemType Directory -Force -Path $probeOutput | Out-Null
cl.exe /nologo /LD /O2 /EHa /W4 /WX tools/tap-foreign-unwind/probe.cpp "/Fo:$probeOutput/probe.obj" "/Fe:$probeOutput/tap_foreign_probe.dll" /link "/IMPLIB:$probeOutput/tap_foreign_probe.lib"
if ($LASTEXITCODE -ne 0) { throw 'Native probe build failed' }
cargo rustc -p collector --example tap_foreign_unwind --features tap-foreign-probe --profile test -- -L "native=$probeOutput"
if ($LASTEXITCODE -ne 0) { throw 'Rust oracle build failed' }
Copy-Item -LiteralPath "$probeOutput/tap_foreign_probe.dll" -Destination .cargo-target/debug/examples/tap_foreign_probe.dll
$env:BATTLE_PLAN_TAP_UNWIND_PROBE = "$probeOutput/tap_foreign_probe.dll"
& ./.cargo-target/debug/examples/tap_foreign_unwind.exe
if ($LASTEXITCODE -ne 0) { throw 'Foreign exception oracle failed' }
```

Require exit zero and four `PASS` lines: normal, C++, SEH and retention. Keep the source head, toolchain versions, profile, PE hashes, commands and output with the run log. Repeat for the final collector profile if it differs; synthetic source assertions or a prior artifact do not establish execution agreement.

Inspect static unwind tables with [`llvm-readobj --unwind`](https://llvm.org/docs/CommandGuide/llvm-readobj.html). For generated code, stop in the native raise export under a Windows debugger; printed native/site/bridge addresses locate the live caller. Retain its disassembly, registered runtime entries and unwind records with that run.

## ABI references

The [Rust FFI guide](https://doc.rust-lang.org/nomicon/ffi.html#ffi-and-unwinding) and [ABI reference](https://doc.rust-lang.org/reference/items/external-blocks.html#r-items.extern.abi.unwind) explain unwind-capable boundaries. Microsoft documents [x64 dispatch](https://learn.microsoft.com/en-us/cpp/build/exception-handling-x64) and the [C++/SEH model](https://learn.microsoft.com/en-us/cpp/build/reference/eh-exception-handling-model). Compile the entire native DLL with `/EHa`; handlers and throwing exports are separate functions, and raise exports must not be declared nonthrowing.
