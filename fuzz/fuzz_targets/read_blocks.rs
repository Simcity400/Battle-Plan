#![no_main]

use libfuzzer_sys::fuzz_target;
use replay::{container::Replay, frame::FrameDecoder, rows::TableId};

fn read(replay: &Replay) {
    for block in &replay.index {
        let mut decoder = FrameDecoder::new();
        if let Ok(frames) = replay.read_block(block) {
            for frame in frames {
                let _ = decoder.advance(&frame);
            }
        }
        let tables = [TableId::Campaign, TableId::Countries, TableId::CountryLinks];
        let _ = replay.read_block_selected(block, &tables);
        let _ = replay.seek_selected(block.branch, block.first_hour, Some(&tables));
        let _ = replay.seek_selected(block.branch, block.last_hour, Some(&tables));
    }
}

fuzz_target!(|data: &[u8]| {
    if let Ok(replay) = Replay::open(data.to_vec()) {
        read(&replay);
    }
    if let Ok(replay) = battle-plan_fuzz::raw_recording(data) {
        read(&replay);
    }
});
