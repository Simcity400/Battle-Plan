use std::{env, fs, io, path::PathBuf};

fn main() -> io::Result<()> {
    let mut path = PathBuf::from("ui/src/generated/dashboard.ts");
    let mut check = false;
    let mut arguments = env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--check" => check = true,
            "--out" => {
                path = arguments
                    .next()
                    .map(PathBuf::from)
                    .ok_or_else(|| io::Error::other("--out needs the contract file's path"))?
            }
            other => return Err(io::Error::other(format!("unknown argument {other}"))),
        }
    }
    let text = dashboard::typescript_contract();
    if check {
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
