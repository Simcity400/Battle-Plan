//! Export synthetic fixture bytes and their Rust-generated contracts to scratch.
use std::{env, fs, path::PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1);
    let output = PathBuf::from(
        args.next()
            .ok_or("usage: wire-fixtures <scratch directory outside Git>")?,
    );
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    fs::create_dir_all(&output)?;
    let output = output.canonicalize()?;
    if output.starts_with(&root) {
        return Err("fixture-output-must-be-outside-git".into());
    }
    let fixtures = wire::fixtures::fixtures()?;
    let manifest = wire::fixtures::manifest(&fixtures)?;
    for fixture in &fixtures {
        fs::write(
            output.join(format!("{}.h4rq", fixture.name)),
            &fixture.bytes,
        )?;
    }
    for fixture in wire::dictionary::synthetic_fixtures()? {
        fs::write(
            output.join(format!("{}.h4rd", fixture.name)),
            &fixture.bytes,
        )?;
    }
    fs::write(
        output.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )?;
    println!(
        "Exported {} Rust-owned fixtures to {}",
        fixtures.len(),
        output.display()
    );
    Ok(())
}
