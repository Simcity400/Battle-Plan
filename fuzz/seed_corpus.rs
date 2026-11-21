
use std::{fs, io, path::Path};

mod fixtures;
use fixtures::recording;

fn main() -> io::Result<()> {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("corpus");
    let valid = recording(false)?;
    let mut seeds = vec![
        ("valid", valid.bytes().to_vec()),
        (
            "snapshot-delta",
            fixtures::recording_sequence(false, true)?.bytes().to_vec(),
        ),
        ("partial", recording(true)?.bytes().to_vec()),
        ("empty", Vec::new()),
        (
            "truncated",
            valid.bytes()[..valid.bytes().len() - 1].to_vec(),
        ),
    ];
    let mut wrong_magic = valid.bytes().to_vec();
    wrong_magic[0] ^= 0xff;
    seeds.push(("wrong-magic", wrong_magic));
    let mut wrong_crc = valid.bytes().to_vec();
    let last = wrong_crc.len() - 1;
    wrong_crc[last] ^= 1;
    seeds.push(("wrong-crc", wrong_crc));
    let mut huge_raw = valid.bytes().to_vec();
    let raw_at = huge_raw.len() - 24 - 8;
    huge_raw[raw_at..raw_at + 8]
        .copy_from_slice(&(replay::container::MAX_BLOCK_RAW_BYTES as u64).to_le_bytes());
    let footer = huge_raw.len() - 24;
    let trailer =
        u64::from_le_bytes(huge_raw[footer + 4..footer + 12].try_into().unwrap()) as usize;
    let checksum = crc32fast::hash(&huge_raw[trailer..footer]);
    huge_raw[footer + 20..].copy_from_slice(&checksum.to_le_bytes());
    seeds.push(("huge-raw", huge_raw));
    let mut huge_count = valid.bytes().to_vec();
    huge_count[trailer + 4..trailer + 8].copy_from_slice(&u32::MAX.to_le_bytes());
    let checksum = crc32fast::hash(&huge_count[trailer..footer]);
    huge_count[footer + 20..].copy_from_slice(&checksum.to_le_bytes());
    seeds.push(("huge-count", huge_count));
    for target in [
        "open_recording",
        "partial_trailer",
        "read_blocks",
        "dashboard_metadata",
    ] {
        let directory = corpus.join(target);
        fs::create_dir_all(&directory)?;
        for (name, bytes) in &seeds {
            fs::write(directory.join(name), bytes)?;
        }
        if matches!(target, "read_blocks" | "dashboard_metadata") {
            let block = &valid.index[0];
            let compressed =
                &valid.bytes()[block.offset as usize..(block.offset + block.length) as usize];
            fs::write(directory.join("raw-frame"), zstd::decode_all(compressed)?)?;
            let sequence = fixtures::recording_sequence(false, true)?;
            let block = &sequence.index[0];
            let compressed =
                &sequence.bytes()[block.offset as usize..(block.offset + block.length) as usize];
            fs::write(
                directory.join("raw-snapshot-delta"),
                zstd::decode_all(compressed)?,
            )?;
        }
    }
    Ok(())
}
