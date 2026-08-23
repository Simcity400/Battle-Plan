//! Lane PV diagnostic: fold one spool and write ONE hour's snapshot bytes.
//! usage: pv_hour_dump <spool> <hour_index> <out.json> [identity]

use std::io::Write;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 {
        eprintln!("usage: pv_hour_dump <spool> <hour_index> <out.json> [identity]");
        std::process::exit(2);
    }
    let target: i64 = args[2].parse().expect("hour_index i64");
    let mode = if args.get(4).map(|value| value == "identity").unwrap_or(false) {
        StreamIdentityModeOf::Identity
    } else {
        StreamIdentityModeOf::detect(&args[1])
    };
    let source = replay_decoder::Source::StreamFile(std::path::PathBuf::from(&args[1]));
    let mut cursor = source
        .open_cursor_with_identity_mode(&replay_decoder::OpenOptions::default(), mode)
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
                    println!("wrote hour {} ({} bytes)", hour.hour_index, hour.snapshot.len());
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
