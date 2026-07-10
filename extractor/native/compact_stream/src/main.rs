use compact_stream::{decode_file, encode_file, StreamCompression};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn main() -> ExitCode {
    match run(std::env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("compact_stream: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: Vec<String>) -> Result<(), String> {
    if matches!(
        args.first().map(String::as_str),
        None | Some("-h") | Some("--help")
    ) {
        print_usage();
        return Ok(());
    }
    if args.len() != 3 {
        return Err(usage());
    }
    let input = PathBuf::from(&args[1]);
    let output = PathBuf::from(&args[2]);
    reject_same_path(&input, &output)?;
    let (decoded, encoded) = match args[0].as_str() {
        "compress" => encode_file(&input, &output, StreamCompression::Zstd),
        "decompress" => decode_file(&input, &output),
        other => return Err(format!("unknown mode {other:?}\n{}", usage())),
    }
    .map_err(|error| format!("{} -> {}: {error}", input.display(), output.display()))?;
    println!(
        "input={} output={} decoded_bytes={} stored_bytes={}",
        input.display(),
        output.display(),
        decoded,
        encoded
    );
    Ok(())
}

fn reject_same_path(input: &Path, output: &Path) -> Result<(), String> {
    let input = std::fs::canonicalize(input)
        .map_err(|error| format!("input {}: {error}", input.display()))?;
    let output = if output.exists() {
        std::fs::canonicalize(output)
            .map_err(|error| format!("output {}: {error}", output.display()))?
    } else {
        let parent = output.parent().unwrap_or_else(|| Path::new("."));
        let parent = std::fs::canonicalize(parent)
            .map_err(|error| format!("output parent {}: {error}", parent.display()))?;
        parent.join(output.file_name().ok_or("output path has no file name")?)
    };
    if input == output {
        return Err("input and output must be different paths".to_string());
    }
    Ok(())
}

fn print_usage() {
    eprintln!("{}", usage());
}

fn usage() -> String {
    "usage:\n  compact_stream compress <input> <output>\n  compact_stream decompress <input> <output>".to_string()
}
