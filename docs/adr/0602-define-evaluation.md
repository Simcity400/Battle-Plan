# Evaluate defines in one restricted Lua state per content stack

Run the stack's define scripts together in PUC Lua 5.1. Scripts in a directory execute before the scripts in its children. Both levels use bytewise order and the suffix check is case-sensitive. A repeated path selects the winning layer while paths that differ only in case remain separate. Shared state lets later mod scripts extend and override earlier scripts. An error refuses the reader, matching a failed content load.

Defines need Lua arithmetic, local functions, tables and loops. Open the base, string, table and math libraries and remove the file loaders. Scripts that need other libraries are refused. A global instruction hook counts coroutines under the same budget. Lua's error handlers can swallow errors, so the protected-call wrappers rethrow budget exhaustion and the state remains spent after it crosses the limit. A separate memory limit bounds runaway strings or tables. Debug output dumps values, which avoids including VM handle addresses.

Typed consumers read `NDefines` values through the matching native conversion. Lists keep Lua iteration order. Integer lookup truncates through the native 64-bit conversion and keeps the low 32 bits. Fixed5 narrows to `f32` first, applies the signed rounding adjustment, and truncates. A double-precision intermediate would change half-step or large-value results.

Simulation sections always come from the recording. Both the map and dashboard can replace presentation sections using visual mods. If those scripts fail, or a mod deletes or retypes a required value, retain the recorded value and name the refusal in content diagnostics. Compare Lua integer and float values as the same type. Copy graphs by source identity so aliases and cycles survive. Use separate copies for recorded and modded sections so a shared table cannot link a modded graphics value back into simulation data.

[Define reader](../../crates/content-reader-defines/src/defines.rs)
