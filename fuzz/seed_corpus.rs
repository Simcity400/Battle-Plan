use std::{
    fs, io,
    path::{Path, PathBuf},
};

use battleplan_fuzz::{fixtures, raw_seed};
use replay_columnar::envelope;

fn main() -> io::Result<()> {
    let corpus = std::env::args_os().nth(1).map_or_else(
        || Path::new(env!("CARGO_MANIFEST_DIR")).join("corpus"),
        PathBuf::from,
    );
    let valid = fixtures::recording(false)?;
    let sequence = fixtures::recording_sequence(false, true)?;
    let partial = fixtures::recording(true)?;
    let mut wrong_magic = valid.bytes().to_vec();
    wrong_magic[0] ^= 0xff;
    let mut damaged_root = valid.bytes().to_vec();
    let (root, _) = envelope::last_root(&damaged_root)?;
    damaged_root[root.offset as usize] ^= 1;
    let damaged_table =
        sequence.damaged_table(&sequence.index[0], replay_data::rows::TableId::Countries)?;
    let seeds = [
        ("valid", valid.bytes()),
        ("snapshot-delta", sequence.bytes()),
        ("partial", partial.bytes()),
        ("empty", &[]),
        ("truncated", &valid.bytes()[..valid.bytes().len() - 1]),
        ("wrong-magic", &wrong_magic),
        ("damaged-root", &damaged_root),
        ("damaged-table", &damaged_table),
    ];
    let raw_frame = raw_seed(false)?;
    let raw_sequence = raw_seed(true)?;
    for target in [
        "open_recording",
        "partial_trailer",
        "read_blocks",
        "dashboard_metadata",
    ] {
        let directory = corpus.join(target);
        fs::create_dir_all(&directory)?;
        for (name, bytes) in seeds {
            fs::write(directory.join(name), bytes)?;
        }
        if matches!(target, "read_blocks" | "dashboard_metadata") {
            fs::write(directory.join("raw-frame"), &raw_frame)?;
            fs::write(directory.join("raw-snapshot-delta"), &raw_sequence)?;
        }
    }
    Ok(())
}
