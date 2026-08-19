fn main() {
    println!("cargo:rerun-if-changed=src/seh_read.c");
    let target = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if target != "msvc" {
        println!("cargo:warning=seh_read.c skipped (non-msvc target_env)");
        return;
    }
    let static_crt = std::env::var("CARGO_CFG_TARGET_FEATURE")
        .unwrap_or_default()
        .split(',')
        .any(|feature| feature == "crt-static");
    cc::Build::new()
        .file("src/seh_read.c")
        // Match the Rust target exactly. Central Windows Release validation
        .static_crt(static_crt)
        .compile("seh_read");
}
