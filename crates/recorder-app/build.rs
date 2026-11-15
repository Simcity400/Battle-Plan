fn main() {
    // Without the embedded UI the panel loads the dev server, never draws, and so never shows.
    if tauri_build::is_dev() && std::env::var("PROFILE").as_deref() == Ok("release") {
        panic!(
            "a release battle-plan-recorder must embed its UI: build with --features recorder-app/custom-protocol"
        );
    }
    tauri_build::build();
}
