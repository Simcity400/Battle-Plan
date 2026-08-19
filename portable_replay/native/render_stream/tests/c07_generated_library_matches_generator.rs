//! because the generated `.rs` had grown a hand-written provenance block the
//! `c07_trees.rs`, the generated file is byte-reproducible, and this target runs
//! Scope: this is a GENERATED-ARTIFACT gate. Whether the library's content is

use std::path::PathBuf;
use std::process::Command;

use portable_render_stream::{
    C07_TREE_CONTENT_LIBRARY_BUCKETS, C07_TREE_CONTENT_LIBRARY_CELLS,
    C07_TREE_CONTENT_LIBRARY_INSTANCE_BYTES, C07_TREE_CONTENT_LIBRARY_PACKED_BYTES_SHA256,
    C07_TREE_CONTENT_LIBRARY_PAYLOAD_SHA256,
};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .expect("render_stream manifest dir has a repository root")
        .to_path_buf()
}

fn python() -> &'static str {
    const CANDIDATES: [&str; 3] = ["python", "python3", "py"];
    for candidate in CANDIDATES {
        let probe = Command::new(candidate).arg("--version").output();
        if matches!(probe, Ok(ref output) if output.status.success()) {
            return candidate;
        }
    }
    panic!(
        "no Python interpreter on PATH (tried {CANDIDATES:?}); the C07 generated-artifact \
         gate cannot run, and this repository already requires Python for central validation"
    );
}

fn receipt_scalar<'a>(receipt: &'a str, key: &str) -> &'a str {
    let needle = format!("\"{key}\": ");
    let mut hits = receipt.lines().filter_map(|line| {
        line.trim_start()
            .strip_prefix(needle.as_str())
            .map(|rest| rest.trim_end_matches(',').trim_matches('"'))
    });
    let value = hits
        .next()
        .unwrap_or_else(|| panic!("generator receipt has no `{key}` field:\n{receipt}"));
    assert!(
        hits.next().is_none(),
        "generator receipt reports `{key}` more than once; the assertions below \
         would be reading an arbitrary one of them"
    );
    value
}

#[test]
fn generated_c07_tree_library_still_matches_the_exact_producer() {
    let repo = repo_root();
    let script = repo
        .join("replay_interface/re_evidence/rendering_laws/C07/build_c07_portable_tree_library.py");
    assert!(
        script.is_file(),
        "the C07 tree-library generator is missing: {}. It is the sole authority \
         for the three generated artifacts this crate compiles in; without it \
         nothing re-derives them.",
        script.display()
    );

    // A zero exit from a generator that compared nothing would be indistinguishable
    // from a pass, so pin that the artifacts it is asked to check are really here
    // and really populated before believing anything it says.
    let src = repo.join("portable_replay/native/render_stream/src");
    let packed = std::fs::read(src.join("c07_tree_content_library.bin"))
        .expect("the packed C07 instance library must be readable");
    assert_eq!(
        packed.len(),
        C07_TREE_CONTENT_LIBRARY_INSTANCE_BYTES as usize,
        "c07_tree_content_library.bin is {} bytes, not the {} the crate's own \
         constants require",
        packed.len(),
        C07_TREE_CONTENT_LIBRARY_INSTANCE_BYTES
    );
    let buckets = std::fs::read_to_string(src.join("c07_tree_complete_buckets.rs"))
        .expect("the generated bucket table must be readable");
    let bucket_rows = buckets.matches("ExpectedLibraryBucket {").count();
    assert_eq!(
        bucket_rows, C07_TREE_CONTENT_LIBRARY_BUCKETS,
        "the generated bucket table holds {bucket_rows} rows, not the \
         {C07_TREE_CONTENT_LIBRARY_BUCKETS} this crate's constants require"
    );
    let hashes = std::fs::read_to_string(src.join("c07_tree_content_hashes.rs"))
        .expect("the generated pin file must be readable");
    for pin in [
        C07_TREE_CONTENT_LIBRARY_PACKED_BYTES_SHA256,
        C07_TREE_CONTENT_LIBRARY_PAYLOAD_SHA256,
    ] {
        assert!(
            hashes.contains(pin),
            "c07_tree_content_hashes.rs no longer carries the pin {pin} the crate \
             compiled; the file on disk is not the file that was included"
        );
    }

    let mut command = Command::new(python());
    command.arg(&script).arg("--verify");
    // The generator defaults to this host's install path. An explicit override
    // is honoured so the gate is runnable where the install lives elsewhere; it
    // is deliberately not a way to opt out, because the generator's own
    // game-file identity gate rejects anything but the pinned vanilla bytes.
    if let Some(root) = std::env::var_os("BATTLE_PLAN_GAME_ROOT").or_else(|| std::env::var_os("HOI4_GAME_ROOT")) {
        command.arg("--game-root").arg(root);
    }
    let output = command
        .output()
        .expect("run the C07 tree-library generator in --verify mode");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "the committed C07 tree-library artifacts no longer match the exact producer.\n\
         Re-derive them with `python {}` and review the diff: a changed pin literal \
         means the canonical library envelope moved, and a changed bucket row or \
         .bin byte means library CONTENT moved, which is a finding and not a \
         re-stamp. Never hand-edit a generated file or a pin literal.\n\
         --- generator stderr ---\n{}\n--- generator stdout ---\n{}",
        script.display(),
        stderr.trim(),
        stdout.trim(),
    );

    // A zero exit is not the success signal. Bind to what the generator says it
    // did: which artifacts it compared, and the two digests it re-derived from
    // the installed content, which must equal the literals this test binary compiled.
    // If `emit` ever stopped comparing, or `--verify` silently became a no-op,
    // the exit code would not move but these would.
    assert_eq!(
        receipt_scalar(&stdout, "mode"),
        "verify",
        "the generator did not report running in verify mode; a generate run \
         would have rewritten the artifacts instead of checking them"
    );
    assert_eq!(
        receipt_scalar(&stdout, "packed_sha256"),
        C07_TREE_CONTENT_LIBRARY_PACKED_BYTES_SHA256,
        "the exact producer's packed-byte digest differs from the pin this crate \
         compiled"
    );
    assert_eq!(
        receipt_scalar(&stdout, "content_library_payload_sha256"),
        C07_TREE_CONTENT_LIBRARY_PAYLOAD_SHA256,
        "the exact producer's canonical-envelope digest differs from the pin this \
         crate compiled"
    );
    assert_eq!(
        receipt_scalar(&stdout, "bucket_count"),
        C07_TREE_CONTENT_LIBRARY_BUCKETS.to_string(),
        "the exact producer emitted a different bucket count"
    );
    assert_eq!(
        receipt_scalar(&stdout, "cell_count"),
        C07_TREE_CONTENT_LIBRARY_CELLS.to_string(),
        "the exact producer emitted a different cell count"
    );
    assert_eq!(
        receipt_scalar(&stdout, "packed_bytes"),
        C07_TREE_CONTENT_LIBRARY_INSTANCE_BYTES.to_string(),
        "the exact producer emitted a different packed-byte total"
    );

    // `verified_outputs` is the list of artifacts the generator actually read
    // back and found identical. All three must be in it: a run that checked two
    // of three still exits zero.
    let verified: Vec<&str> = stdout
        .lines()
        .skip_while(|line| line.trim() != "\"verified_outputs\": [")
        .skip(1)
        .take_while(|line| !line.trim().starts_with(']'))
        .map(|line| line.trim().trim_end_matches(',').trim_matches('"'))
        .collect();
    assert_eq!(
        verified,
        vec![
            "c07_tree_complete_buckets.rs",
            "c07_tree_content_hashes.rs",
            "c07_tree_content_library.bin",
        ],
        "the generator did not report comparing all three generated artifacts; \
         it reported {verified:?}"
    );
    assert!(
        stdout.contains("\"changed_outputs\": [],"),
        "the generator reported writing something in --verify mode, which it must \
         never do:\n{stdout}"
    );
}
