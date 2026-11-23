# 0310. Preserve exported dashboard field documentation as metadata

The dashboard TypeScript contract includes field and variant documentation in the declarations produced by `ts-rs`. The export check compares the generated file byte for byte. Rust line documentation already expands to a `doc` attribute before the derive sees it. Keep the exact attribute values in source for verified export consumers so their generated descriptions and Rust documentation remain stable.

Each original line remains a separate `doc` attribute in the same order. The generator formats a single multiline value differently from a sequence of values. Decisions about why a field is timed, withheld, or encoded a particular way are recorded in the relevant subsystem ADRs.

[Dashboard type export](../../crates/dashboard/src/contract.rs) · [Generated contract](../../ui/src/generated/dashboard.ts) · [Export check](../../tools/smoke.ps1)
