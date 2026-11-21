
use engine_executable::{build::ExecutableIdentity, executable_tables};
use std::{path::PathBuf, time::Instant};

fn main() -> std::io::Result<()> {
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    if arguments.first().is_some_and(|flag| flag == "--pin") && arguments.len() == 2 {
        let bytes = std::fs::read(PathBuf::from(&arguments[1]).join("hoi4.exe"))?;
        if !ExecutableIdentity::of(&bytes).recognised() {
            eprintln!("--pin needs the recognised build");
            std::process::exit(2);
        }
        print!("{}", executable_tables::registry_identities_source(bytes)?);
        return Ok(());
    }
    let folders: Vec<PathBuf> = arguments.into_iter().map(PathBuf::from).collect();
    if folders.is_empty() {
        eprintln!("usage: executable-tables-check <game folder>... | --pin <game folder>");
        std::process::exit(2);
    }
    let mut withheld = false;
    for folder in folders {
        let bytes = std::fs::read(folder.join("hoi4.exe"))?;
        let identity = ExecutableIdentity::of(&bytes);
        let started = Instant::now();
        let tables = executable_tables::decode(identity, bytes, &folder);
        println!(
            "{}: {} in {:.0?}",
            folder.display(),
            if tables.recognised() { "recognised" } else { "unrecognised" },
            started.elapsed()
        );
        match &tables.version {
            Ok(version) => println!("  version: {version}"),
            Err(error) => println!("  {error}"),
        }
        for (name, digest) in tables.content_digests() {
            match digest {
                Ok(digest) => println!("  {name}: verified, content {}", hex(&digest)),
                Err(error) => {
                    withheld = true;
                    println!("  {error}");
                }
            }
        }
        for withheld_part in tables.withheld() {
            withheld = true;
            println!("  withheld: {withheld_part}");
        }
        if let Some(warning) = tables.build_warning() {
            println!("  {warning}");
        }
    }
    std::process::exit(i32::from(withheld));
}

fn hex(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
