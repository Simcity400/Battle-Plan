fn main() {
    if tauri_build::is_dev() && std::env::var("PROFILE").as_deref() == Ok("release") {
        panic!(
            "a release battle-plan-recorder must embed its UI: build with --features recorder-app/custom-protocol"
        );
    }
    tauri_build::build();
}
