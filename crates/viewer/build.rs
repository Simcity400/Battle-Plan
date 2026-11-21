use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn main() {
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=Cargo.toml");
    for name in ["HEAD", "index", "packed-refs", "refs/remotes/origin/main"] {
        if let Some(path) = git(&["rev-parse", "--git-path", name]) {
            println!("cargo:rerun-if-changed={path}");
        }
    }
    if let Some(reference) = git(&["symbolic-ref", "-q", "HEAD"])
        && let Some(path) = git(&["rev-parse", "--git-path", &reference])
    {
        println!("cargo:rerun-if-changed={path}");
    }
    if let Some(commit) = git(&["rev-parse", "HEAD"]) {
        println!("cargo:rustc-env=BATTLEPLAN_PLAYBACK_COST_COMMIT={commit}");
    }
    if let Some(base) = git(&["merge-base", "HEAD", "origin/main"]) {
        println!("cargo:rustc-env=BATTLEPLAN_PLAYBACK_COST_MAIN={base}");
    }
    if let Some(status) = git(&["status", "--porcelain", "--untracked-files=normal"]) {
        println!(
            "cargo:rustc-env=BATTLEPLAN_PLAYBACK_COST_DIRTY={}",
            !status.is_empty()
        );
    }
}
