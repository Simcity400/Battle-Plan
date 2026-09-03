//! Lane PV diagnostic: fold one spool and write ONE hour's snapshot bytes.
//! usage: pv_hour_dump <spool> <hour_index> <out.json> [identity] [--fold-cache <dir>] [--game-id <id>]
//! With `--fold-cache`, the spool's cached certified fold is adopted when it
//! is valid for this binary and the hour is reconstructed from it in seconds;
//! otherwise the spool is folded from the start as before. Whichever route
//! served the hour is named on stdout.

use std::io::{Read, Write};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 {
        eprintln!(
            "usage: pv_hour_dump <spool> <hour_index> <out.json> [identity] [--fold-cache <dir>] [--game-id <id>]"
        );
        std::process::exit(2);
    }
    let target: i64 = args[2].parse().expect("hour_index i64");
    let mut identity = false;
    let mut fold_cache: Option<std::path::PathBuf> = None;
    let mut game_id: Option<String> = None;
    let mut i = 4;
    while i < args.len() {
        match args[i].as_str() {
            "identity" => identity = true,
            "--game-id" => {
                i += 1;
                game_id = Some(args.get(i).expect("--game-id requires a value").clone());
            }
            "--fold-cache" => {
                i += 1;
                fold_cache = Some(std::path::PathBuf::from(
                    args.get(i).expect("--fold-cache requires a path"),
                ));
            }
            other => {
                eprintln!("unknown argument {other:?}");
                std::process::exit(2);
            }
        }
        i += 1;
    }
    let mode = if identity {
        StreamIdentityModeOf::Identity
    } else {
        StreamIdentityModeOf::detect(&args[1])
    };
    if let Some(root) = fold_cache.as_deref() {
        let (source_len, source_sha256) = hash_file(std::path::Path::new(&args[1]));
        let params = replay_decoder::fold_cache::FoldCacheParams::new(
            game_id
                .as_deref()
                .expect("--fold-cache needs --game-id: cache entries are keyed by the fold's game id"),
            mode,
            snapshot::DEFAULT_CHECKPOINT_INTERVAL_HOURS,
        );
        match replay_decoder::fold_cache::open_reader(root, source_len, &source_sha256, &params) {
            Ok(Some((reader, hours))) => {
                if !hours.contains(&target) {
                    panic!(
                        "hour {target} not in the cached fold: {} hours, first {:?}, last {:?}",
                        hours.len(),
                        hours.first(),
                        hours.last()
                    );
                }
                let (bytes, _receipt) = reader
                    .reconstruct_json(target)
                    .unwrap_or_else(|error| panic!("reconstruct hour {target} from cache: {error}"));
                let mut file = std::fs::File::create(&args[3]).expect("create out");
                file.write_all(&bytes).expect("write out");
                println!(
                    "wrote hour {target} ({} bytes) from fold cache {}",
                    bytes.len(),
                    replay_decoder::fold_cache::entry_dir(root, &source_sha256, &params).display()
                );
                return;
            }
            Ok(None) => eprintln!("fold cache has no entry for this spool and fold; folding"),
            Err(reason) => eprintln!("fold cache entry refused ({reason}); folding"),
        }
    }
    let source = replay_decoder::Source::StreamFile(std::path::PathBuf::from(&args[1]));
    let options = replay_decoder::OpenOptions {
        game_id: game_id.clone(),
    };
    let mut cursor = source
        .open_cursor_with_identity_mode(&options, mode)
        .unwrap_or_else(|error| panic!("open: {error}"));
    let mut count = 0u64;
    let mut first: Option<i64> = None;
    let mut last: Option<i64> = None;
    loop {
        match cursor.next_hour() {
            Some(Ok(hour)) => {
                count += 1;
                first.get_or_insert(hour.hour_index);
                last = Some(hour.hour_index);
                if hour.hour_index == target {
                    let mut file = std::fs::File::create(&args[3]).expect("create out");
                    file.write_all(&hour.snapshot).expect("write out");
                    println!(
                        "wrote hour {} ({} bytes) by folding the spool",
                        hour.hour_index,
                        hour.snapshot.len()
                    );
                    return;
                }
            }
            Some(Err(error)) => panic!("fold after {} hours: {error}", count),
            None => panic!(
                "hour {target} not found: {count} hours emitted, first {:?}, last {:?}",
                first, last
            ),
        }
    }
}

fn hash_file(path: &std::path::Path) -> (u64, String) {
    use sha2::Digest;
    let mut file = std::fs::File::open(path).expect("open spool");
    let mut hasher = sha2::Sha256::new();
    let mut buffer = vec![0u8; 1 << 20];
    let mut total = 0u64;
    loop {
        let read = file.read(&mut buffer).expect("read spool");
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        total += read as u64;
    }
    (total, format!("{:x}", hasher.finalize()))
}

mod StreamIdentityModeOf {
    use replay_decoder::StreamIdentityMode;
    pub const Identity: StreamIdentityMode = StreamIdentityMode::IdentityV1;
    pub fn detect(path: &str) -> StreamIdentityMode {
        use std::io::Read;
        let mut file = std::fs::File::open(path).expect("open");
        let mut head = [0u8; 40];
        let read = file.read(&mut head).expect("read");
        if read >= 24
            && &head[0..4] == compact_contract::COMPACT_RECORD_MAGIC
            && u16::from_le_bytes([head[22], head[23]])
                == compact_contract::COMPACT_RECORD_TYPE_STREAM_HEADER
        {
            StreamIdentityMode::IdentityV1
        } else {
            StreamIdentityMode::Legacy
        }
    }
}
