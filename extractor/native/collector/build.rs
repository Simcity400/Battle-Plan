// Compiles the SEH-guarded crash-safe read shim (src/seh_read.c) into the
// collector cdylib. The shim provides `seh_guarded_copy`, an in-process copy
// wrapped in a structured-exception frame so a decommit-mid-read faults closed
// instead of taking the host game process down. See src/seh_read.c and the
// warm-perf plan v2 (runs/perf_rearch_20260706/).
//
// MSVC only (the collector targets x86_64-pc-windows-msvc and is injected into
// hoi4.exe); __try/__except is an MSVC extension. `cc` locates the MSVC
// toolchain via vswhere (no cl.exe on PATH required).
fn main() {
    println!("cargo:rerun-if-changed=src/seh_read.c");
    let target = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if target != "msvc" {
        // Non-MSVC (e.g. a cross/clippy pass on another env): skip the shim.
        // The Rust FFI fallback in lib.rs handles absence at runtime.
        println!("cargo:warning=seh_read.c skipped (non-msvc target_env)");
        return;
    }
    cc::Build::new()
        .file("src/seh_read.c")
        // Match the Rust side's dynamic CRT (msvcrt) so there is one CRT.
        .static_crt(false)
        .compile("seh_read");
}
