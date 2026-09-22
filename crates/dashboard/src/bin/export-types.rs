use std::{fs, io, path::PathBuf};

fn main() -> io::Result<()> {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../ui/src/generated/dashboard.ts");
    let text = dashboard::typescript_contract();
    if std::env::args().any(|argument| argument == "--check") {
        if fs::read_to_string(&path)? != text {
            return Err(io::Error::other(
                "dashboard TypeScript contract needs regeneration",
            ));
        }
    } else {
        fs::create_dir_all(path.parent().expect("contract has a parent"))?;
        fs::write(path, text)?;
    }
    Ok(())
}
