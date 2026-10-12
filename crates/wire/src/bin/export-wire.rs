//! Generate the TypeScript contracts without touching the existing JSON path.
use std::{env, fs, path::PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = env::args().skip(1).collect();
    let check = args.iter().any(|s| s == "--check");
    let output = args
        .iter()
        .position(|s| s == "--output")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../ui/src/generated/wire.ts")
        });
    let source = wire::shapes::typescript();
    if check {
        if fs::read_to_string(&output)? != source {
            return Err("generated-wire-contract-mismatch".into());
        }
    } else {
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(output, source)?;
    }
    Ok(())
}
