// Decides this crate's test preconditions once, at build time, and records
// each as a `cfg`. See `test_gates.rs` and the `replay_test_gates` crate docs.
include!("test_gates.rs");

fn main() {
    println!("cargo::rerun-if-changed=test_gates.rs");
    replay_test_gates::emit(GATES);
}
