use std::{
    io::{self, Cursor},
    sync::OnceLock,
};

use replay::campaign::{Replay, SeekResult};
use replay_columnar::{
    container::{ContainerWriter, SectionKey, Tags},
    envelope::Kind,
    pages::Block,
    unit::{FrameEntry, restore},
};
use replay_data::rows::{TableId, tables::Countries};

#[path = "../fixtures.rs"]
pub mod fixtures;

const LEVEL: i32 = 3;

fn base(sequence: bool) -> &'static Replay {
    static SINGLE: OnceLock<Replay> = OnceLock::new();
    static SEQUENCE: OnceLock<Replay> = OnceLock::new();
    let cell = if sequence { &SEQUENCE } else { &SINGLE };
    cell.get_or_init(|| fixtures::recording_sequence(false, sequence).expect("synthetic recording"))
}

fn country_frame(block: &mut Block) -> &mut FrameEntry {
    let table = &mut block.tables[TableId::Countries.index()].unit;
    if let Some(frame) = table
        .parts
        .first_mut()
        .and_then(|part| part.frames.first_mut())
    {
        return frame;
    }
    &mut table
        .first
        .as_mut()
        .and_then(|first| {
            first
                .streams
                .iter_mut()
                .find(|(stream, _)| *stream == restore::ROWS)
        })
        .and_then(|(_, chunks)| chunks.first_mut())
        .expect("the initial synthetic country snapshot has a rows stream")
        .frame
}

pub fn raw_seed(sequence: bool) -> io::Result<Vec<u8>> {
    let recording = base(sequence).recording();
    let mut block = recording.block(0, 0)?;
    let mut seed = vec![u8::from(sequence)];
    seed.extend(recording.frame(country_frame(&mut block))?);
    let hour = i32::from(sequence);
    let SeekResult::Frame(frame) =
        raw_recording(&seed)?.seek_selected(1, hour, Some(&[TableId::Countries]))?
    else {
        return Err(io::Error::other(
            "the raw seed does not reach its last hour",
        ));
    };
    if frame
        .complete::<Countries>()?
        .first()
        .map(|row| row.command_power_raw)
        != Some(i64::from(hour))
    {
        return Err(io::Error::other(
            "the raw seed does not preserve the synthetic country",
        ));
    }
    Ok(seed)
}

pub fn raw_recording(data: &[u8]) -> io::Result<Replay> {
    let (&shape, raw) = data.split_first().unwrap_or((&0, &[]));
    let base = base(shape & 1 != 0);
    let recording = base.recording();
    let mut block = recording.block(0, 0)?;
    let mut directory = recording.run(0)?.directory().clone();
    let mut writer = ContainerWriter::resume(
        Cursor::new(base.bytes().to_vec()),
        recording.root().clone(),
        recording.revision(),
        LEVEL,
        None,
    )?;
    let extent = writer.put_zstd(Kind::Block, Tags::default(), raw, LEVEL)?;
    let frame = country_frame(&mut block);
    frame.offset = extent.offset;
    frame.stored = u32::try_from(extent.stored).map_err(io::Error::other)?;
    frame.raw = u32::try_from(extent.raw).map_err(io::Error::other)?;
    frame.digest = extent.digest;

    block.head.chain = Block::chain_after(&[0; 32], &block.body()?);
    directory.chain = block.head.chain;
    let tags = Tags {
        hours: u32::from(block.head.hours),
        ..Tags::default()
    };
    directory.open[0] = writer.put_zstd(Kind::BlockPage, tags, &block.write()?, LEVEL)?;
    let run = writer.put_zstd(
        Kind::RunDirectory,
        Tags::default(),
        &directory.write(0)?,
        LEVEL,
    )?;
    writer.set_open(SectionKey::WORLD_LOG, &[run])?;
    let (bytes, _, _) = writer.finish(&[], true, |_| Ok(()))?;
    Replay::open(bytes.into_inner())
}

#[cfg(test)]
mod tests {
    use replay::campaign::SeekResult;
    use replay_data::rows::tables::Countries;

    #[test]
    fn raw_wrapper_reaches_matching_multiframe_snapshot_delta_baselines() {
        for sequence in [false, true] {
            let seed = super::raw_seed(sequence).unwrap();
            let recording = super::raw_recording(&seed).unwrap();
            let last = i32::from(sequence);
            assert_eq!(
                (recording.index[0].first_hour, recording.index[0].last_hour),
                (0, last)
            );
            for hour in 0..=last {
                let SeekResult::Frame(frame) = recording.seek(1, hour).unwrap() else {
                    panic!("synthetic hour is unreachable");
                };
                assert_eq!(
                    frame.complete::<Countries>().unwrap()[0].command_power_raw,
                    i64::from(hour)
                );
            }

            let damaged = super::raw_recording(&seed[..1]).unwrap();
            let SeekResult::Frame(frame) = damaged.seek(1, last).unwrap() else {
                panic!("damaged frame lost its outer recording");
            };
            assert!(frame.complete::<Countries>().is_err());
        }
    }
}
