#[path = "../app-update/src/label.rs"]
mod label;

fn main() {
    if tauri_build::is_dev() && std::env::var("PROFILE").as_deref() == Ok("release") {
        panic!(
            "a release battleplan-recorder must embed its UI: build with --features recorder-app/custom-protocol"
        );
    }
    println!("cargo:rerun-if-env-changed=BATTLEPLAN_VERSION");
    let version = std::env::var("BATTLEPLAN_VERSION").unwrap_or_default();
    let mut config: serde_json::Value = match std::env::var("TAURI_CONFIG") {
        Ok(text) if !text.trim().is_empty() => serde_json::from_str(&text).expect("TAURI_CONFIG holds JSON"),
        _ => serde_json::json!({}),
    };
    config["productName"] = format!("BattlePlan Recorder ({})", label::channel_label(&version)).into();
    // SAFETY: the build script runs no other threads, and tauri_build reads the variable after this.
    unsafe { std::env::set_var("TAURI_CONFIG", config.to_string()) };
    tauri_build::build();
}
