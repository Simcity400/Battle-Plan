use std::env;
use std::fs;
use std::path::PathBuf;

use land_combat_detail_events_live_lane::{gate_stream_for_product, result_json};

const USAGE: &str = "usage: land_combat_detail_stream_validate --stream <compact.bin> --minimum-game-hour-advancements <positive integer>";

#[derive(Debug, PartialEq, Eq)]
struct Args {
    stream: PathBuf,
    minimum_game_hour_advancements: i64,
}

fn parse_args<I>(args: I) -> Result<Args, String>
where
    I: IntoIterator<Item = String>,
{
    let mut stream = None;
    let mut minimum_game_hour_advancements = None;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--stream" => {
                if stream.is_some() {
                    return Err("--stream may be specified only once".to_string());
                }
                stream = Some(PathBuf::from(
                    args.next()
                        .ok_or_else(|| "--stream requires a path".to_string())?,
                ));
            }
            "--minimum-game-hour-advancements" => {
                if minimum_game_hour_advancements.is_some() {
                    return Err(
                        "--minimum-game-hour-advancements may be specified only once".to_string(),
                    );
                }
                let raw = args.next().ok_or_else(|| {
                    "--minimum-game-hour-advancements requires a positive integer".to_string()
                })?;
                let parsed = raw.parse::<i64>().map_err(|_| {
                    format!("--minimum-game-hour-advancements value {raw:?} is not an integer")
                })?;
                if parsed <= 0 {
                    return Err(format!(
                        "--minimum-game-hour-advancements must be positive, got {parsed}"
                    ));
                }
                minimum_game_hour_advancements = Some(parsed);
            }
            "--help" | "-h" => return Err(USAGE.to_string()),
            other => return Err(format!("unknown argument {other:?}")),
        }
    }
    Ok(Args {
        stream: stream.ok_or_else(|| "missing required --stream".to_string())?,
        minimum_game_hour_advancements: minimum_game_hour_advancements
            .ok_or_else(|| "missing required --minimum-game-hour-advancements".to_string())?,
    })
}

fn main() {
    let args = match parse_args(env::args().skip(1)) {
        Ok(args) => args,
        Err(error) => {
            eprintln!("{error}\n{USAGE}");
            std::process::exit(2);
        }
    };
    let stream = match fs::read(&args.stream) {
        Ok(stream) => stream,
        Err(error) => {
            eprintln!("read {} failed: {error}", args.stream.display());
            std::process::exit(2);
        }
    };
    let gate = gate_stream_for_product(&stream, args.minimum_game_hour_advancements);
    println!("{}", result_json(&gate));
    std::process::exit(if gate.ok { 0 } else { 1 });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn parser_requires_stream_and_explicit_positive_duration() {
        assert!(parse_args(strings(&[])).is_err());
        assert!(parse_args(strings(&["--stream", "capture.bin"])).is_err());
        assert!(parse_args(strings(&[
            "--stream",
            "capture.bin",
            "--minimum-game-hour-advancements",
            "0",
        ]))
        .is_err());
        assert_eq!(
            parse_args(strings(&[
                "--stream",
                "capture.bin",
                "--minimum-game-hour-advancements",
                "9",
            ]))
            .unwrap(),
            Args {
                stream: PathBuf::from("capture.bin"),
                minimum_game_hour_advancements: 9,
            }
        );
    }

    #[test]
    fn parser_rejects_duplicates_unknowns_and_non_integer_duration() {
        assert!(parse_args(strings(&[
            "--stream",
            "a",
            "--stream",
            "b",
            "--minimum-game-hour-advancements",
            "1",
        ]))
        .is_err());
        assert!(parse_args(strings(&[
            "--stream",
            "a",
            "--minimum-game-hour-advancements",
            "1.5",
        ]))
        .is_err());
        assert!(parse_args(strings(&["--wat"])).is_err());
    }
}
