#![no_main]

use libfuzzer_sys::fuzz_target;
use replay::campaign::Replay;
use replay_data::rows::TableId;

fn read(replay: &Replay) {
    for block in &replay.index {
        let _ = replay.read_block_alone(block, &TableId::ALL, |_, _| Ok(()), || Ok(()));
        let tables = [TableId::Campaign, TableId::Countries, TableId::CountryLinks];
        let _ = replay.read_block_alone(block, &tables, |_, _| Ok(()), || Ok(()));
        let _ = replay.seek_selected(block.branch, block.first_hour, Some(&tables));
        let _ = replay.seek_selected(block.branch, block.last_hour, Some(&tables));
    }
}

fuzz_target!(|data: &[u8]| {
    if let Ok(replay) = Replay::open(data.to_vec()) {
        read(&replay);
    }
    if let Ok(replay) = battleplan_fuzz::raw_recording(data) {
        read(&replay);
    }
});
