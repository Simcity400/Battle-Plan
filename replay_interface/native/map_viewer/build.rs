include!("test_gates.rs");

fn main() {
    println!("cargo::rerun-if-changed=test_gates.rs");
    replay_test_gates::emit(GATES);
}
