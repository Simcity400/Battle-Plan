//! Extractor-owned production worker for compact-stream pre-folding.
//!
//! The replay product invokes this sibling process so its Cargo graph remains
//! free of extractor/live-lane crates. Input paths name a replay-owned private
//! spool; `replay_decoder` authenticates the exact H4CR bytes and drives the
//! single production WorldFold before this process emits a small JSON receipt.

use std::ffi::OsString;
use std::path::PathBuf;

const RECEIPT_SCHEMA: &str = "hoi4_prefold_worker_receipt_v2";
const DECODER_ID: &str = "extractor.replay_decoder.production_world_fold.worker.v1";

#[derive(Default)]
struct Args {
    game_id: Option<String>,
    source: Option<PathBuf>,
    snapshot_dir: Option<PathBuf>,
    source_len: Option<u64>,
    source_sha256: Option<String>,
    identity_mode: Option<snapshot::PrefoldStreamIdentityMode>,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("replay_prefold_worker: {error}");
        std::process::exit(2);
    }
}

fn run() -> Result<(), String> {
    let args = parse_args(std::env::args_os().skip(1))?;
    let game_id = args.game_id.ok_or("missing --game-id")?;
    let source = args.source.ok_or("missing --source")?;
    let snapshot_dir = args.snapshot_dir.ok_or("missing --snapshot-dir")?;
    let source_len = args.source_len.ok_or("missing --source-len")?;
    let source_sha256 = args.source_sha256.ok_or("missing --source-sha256")?;
    let identity_mode = args.identity_mode.ok_or("missing --identity-mode")?;
    let identity_mode_label = match identity_mode {
        snapshot::PrefoldStreamIdentityMode::Legacy => "legacy",
        snapshot::PrefoldStreamIdentityMode::IdentityV1 => "identity-v1",
    };
    let request = snapshot::CertifiedPrefoldSpoolRequest::try_new_with_identity_mode(
        game_id,
        source,
        snapshot_dir,
        source_len,
        source_sha256,
        identity_mode,
    )
    .map_err(|error| error.to_string())?;
    let spool = replay_decoder::fold_raw_h4cr_file_spooled(request)
        .map_err(|error| error.to_string())?;
    let entries = spool
        .entries()
        .iter()
        .map(|entry| {
            serde_json::json!({
                "hour_index": entry.hour_index(),
                "date": entry.date(),
                "file_name": entry.file_name(),
                "byte_len": entry.byte_len(),
                "sha256": entry.sha256(),
            })
        })
        .collect::<Vec<_>>();
    let receipt = serde_json::json!({
        "schema": RECEIPT_SCHEMA,
        "decoder_id": DECODER_ID,
        "identity_mode": identity_mode_label,
        "game_id": spool.game_id(),
        "schema_version": spool.schema_version(),
        "entries": entries,
    });
    serde_json::to_writer(std::io::stdout().lock(), &receipt)
        .map_err(|error| format!("serialize worker receipt: {error}"))?;
    Ok(())
}

fn parse_args(iter: impl IntoIterator<Item = OsString>) -> Result<Args, String> {
    let mut iter = iter.into_iter();
    let mut parsed = Args::default();
    while let Some(flag) = iter.next() {
        let flag_text = flag
            .to_str()
            .ok_or_else(|| "worker option name is not Unicode".to_string())?;
        let value = iter
            .next()
            .ok_or_else(|| format!("missing value for {flag_text}"))?;
        match flag_text {
            "--game-id" => {
                if parsed.game_id.is_some() {
                    return Err("duplicate --game-id".into());
                }
                parsed.game_id = Some(
                    value
                        .into_string()
                        .map_err(|_| "--game-id is not Unicode".to_string())?,
                );
            }
            "--source" => {
                if parsed.source.replace(PathBuf::from(value)).is_some() {
                    return Err("duplicate --source".into());
                }
            }
            "--snapshot-dir" => {
                if parsed.snapshot_dir.replace(PathBuf::from(value)).is_some() {
                    return Err("duplicate --snapshot-dir".into());
                }
            }
            "--source-len" => {
                if parsed.source_len.is_some() {
                    return Err("duplicate --source-len".into());
                }
                let text = value
                    .into_string()
                    .map_err(|_| "--source-len is not Unicode".to_string())?;
                parsed.source_len = Some(
                    text.parse::<u64>()
                        .map_err(|error| format!("invalid --source-len: {error}"))?,
                );
            }
            "--source-sha256" => {
                if parsed.source_sha256.is_some() {
                    return Err("duplicate --source-sha256".into());
                }
                parsed.source_sha256 = Some(
                    value
                        .into_string()
                        .map_err(|_| "--source-sha256 is not Unicode".to_string())?,
                );
            }
            "--identity-mode" => {
                if parsed.identity_mode.is_some() {
                    return Err("duplicate --identity-mode".into());
                }
                let text = value
                    .into_string()
                    .map_err(|_| "--identity-mode is not Unicode".to_string())?;
                parsed.identity_mode = Some(match text.as_str() {
                    "legacy" => snapshot::PrefoldStreamIdentityMode::Legacy,
                    "identity-v1" => snapshot::PrefoldStreamIdentityMode::IdentityV1,
                    _ => {
                        return Err(format!(
                            "invalid --identity-mode {text:?}; expected legacy or identity-v1"
                        ))
                    }
                });
            }
            other => return Err(format!("unknown option {other}")),
        }
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parser_rejects_unknown_and_duplicate_options() {
        assert!(parse_args([OsString::from("--unknown"), OsString::from("x")]).is_err());
        assert!(parse_args([
            OsString::from("--game-id"),
            OsString::from("g"),
            OsString::from("--game-id"),
            OsString::from("g2"),
        ])
        .is_err());
        assert!(parse_args([
            OsString::from("--identity-mode"),
            OsString::from("legacy"),
            OsString::from("--identity-mode"),
            OsString::from("identity-v1"),
        ])
        .is_err());
        assert!(parse_args([
            OsString::from("--identity-mode"),
            OsString::from("sniff"),
        ])
        .is_err());
    }

    #[test]
    fn parser_accepts_both_explicit_identity_modes() {
        for (wire, expected) in [
            ("legacy", snapshot::PrefoldStreamIdentityMode::Legacy),
            (
                "identity-v1",
                snapshot::PrefoldStreamIdentityMode::IdentityV1,
            ),
        ] {
            let parsed = parse_args([
                OsString::from("--identity-mode"),
                OsString::from(wire),
            ])
            .unwrap();
            assert_eq!(parsed.identity_mode, Some(expected));
        }
    }
}
