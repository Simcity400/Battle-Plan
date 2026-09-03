//! Lane PR: run THE attach-time content census over the real install a sealed
//! manifest names, and report mount/walk timings plus row equality against the
//! manifest's own `required_content`.
//! usage: pr_census_profile <native_session_manifest.json> <report.json> [--cache-dir <dir>]

use std::path::PathBuf;
use std::time::Instant;

use portable_content_manifest::{RecordedContentLayer, RequiredContent};
use recorder_host::portable_content_manifest::RuntimeContentRoots;
use serde_json::Value;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!(
            "usage: pr_census_profile <native_session_manifest.json> <report.json> [--cache-dir <dir>]"
        );
        std::process::exit(2);
    }
    let manifest_path = PathBuf::from(&args[1]);
    let report_path = PathBuf::from(&args[2]);
    let mut cache_dir: Option<PathBuf> = None;
    let mut i = 3;
    while i < args.len() {
        match args[i].as_str() {
            "--cache-dir" => {
                i += 1;
                cache_dir = Some(PathBuf::from(args.get(i).expect("--cache-dir requires a path")));
            }
            other => {
                eprintln!("unknown argument {other}");
                std::process::exit(2);
            }
        }
        i += 1;
    }

    let manifest: Value = serde_json::from_slice(
        &std::fs::read(&manifest_path).expect("read sealed manifest"),
    )
    .expect("parse sealed manifest");
    let exe_path = PathBuf::from(
        manifest["modset"]["game"]["exe_path"]
            .as_str()
            .expect("modset.game.exe_path"),
    );
    let user_dir = PathBuf::from(
        manifest["modset"]["pdx_user_dir"]
            .as_str()
            .expect("modset.pdx_user_dir"),
    );
    let roots = RuntimeContentRoots {
        install_root: exe_path.parent().expect("install root").to_path_buf(),
        user_dir,
    };
    let content = &manifest["portable_content_manifest"];
    let layers: Vec<RecordedContentLayer> =
        serde_json::from_value(content["ordered_layers"].clone()).expect("ordered_layers");
    let authority_sha256 = content["effective_vfs_manifest_sha256"]
        .as_str()
        .expect("effective_vfs_manifest_sha256")
        .to_string();
    let sealed_rows: Vec<RequiredContent> =
        serde_json::from_value(content["required_content"].clone()).expect("required_content");

    let mount_started = Instant::now();
    let vfs = recorder_host::production_content_vfs::effective_vfs_from_launch(
        &roots,
        &layers,
        &authority_sha256,
    )
    .expect("mount effective VFS");
    let mount_seconds = mount_started.elapsed().as_secs_f64();

    let census_started = Instant::now();
    let census = recorder_host::portable_content_manifest::census_effective_vfs_corpus(
        &vfs,
        cache_dir.as_deref(),
    );
    let census_seconds = census_started.elapsed().as_secs_f64();
    let peak = recorder_host::proc_mem::peak_memory();

    let (rows_json, equality) = match &census {
        Ok((rows, stats)) => {
            let first_mismatch = sealed_rows
                .iter()
                .zip(rows.iter())
                .find(|(sealed, fresh)| sealed != fresh)
                .map(|(sealed, fresh)| {
                    serde_json::json!({
                        "sealed": sealed.content.normalized_game_relative_path,
                        "fresh": fresh.content.normalized_game_relative_path,
                    })
                });
            (
                serde_json::json!({
                    "rows": rows.len(),
                    "files": stats.files,
                    "payload_bytes": stats.payload_bytes,
                    "walk_ms": stats.elapsed_ms,
                    "excluded_engine_written_sinks": stats.excluded_engine_written_sinks,
                    "cache": stats.cache,
                }),
                serde_json::json!({
                    "sealed_rows": sealed_rows.len(),
                    "rows_equal_sealed_manifest": sealed_rows.len() == rows.len()
                        && first_mismatch.is_none(),
                    "first_mismatch": first_mismatch,
                }),
            )
        }
        Err(error) => (
            serde_json::json!({ "error": error }),
            serde_json::json!({ "rows_equal_sealed_manifest": false }),
        ),
    };
    let report = serde_json::json!({
        "tool": "pr_census_profile",
        "manifest": manifest_path.display().to_string(),
        "install_root": roots.install_root.display().to_string(),
        "layers": layers.len(),
        "cache_dir": cache_dir.as_ref().map(|dir| dir.display().to_string()),
        "mount_seconds": mount_seconds,
        "census_seconds": census_seconds,
        "peak_private_bytes": peak.as_ref().map(|p| p.peak_private_bytes),
        "peak_working_set_bytes": peak.as_ref().map(|p| p.peak_working_set_bytes),
        "census": rows_json,
        "equality": equality,
    });
    let text = serde_json::to_string_pretty(&report).expect("report serializes") + "\n";
    std::fs::write(&report_path, text.as_bytes()).expect("write report");
    eprintln!("{text}");
    if census.is_err() {
        std::process::exit(1);
    }
}
