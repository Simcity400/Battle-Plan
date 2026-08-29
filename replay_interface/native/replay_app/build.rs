include!("test_gates.rs");

/// The stack this image reserves, over the 1 MiB Windows would give it.
///
/// Preparing one map foundation frame occupies 990.3 KiB of an unoptimized
/// build's main thread, which the default does not fit;
/// `extractor/native/runs/uy_tree_append/stack_frame_budget_fixed_build.txt` is
/// the measurement. Reserve is address space -- the PE commit stays one page.
const STACK_RESERVE_BYTES: u64 = 8 * 1024 * 1024;

fn main() {
    println!("cargo::rerun-if-changed=test_gates.rs");
    replay_test_gates::emit(GATES);
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo::rustc-link-arg=/STACK:{STACK_RESERVE_BYTES}");
    }
}
