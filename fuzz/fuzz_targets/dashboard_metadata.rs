#![no_main]

use std::{
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool},
};

use dashboard::{Source, SourceKey};
use libfuzzer_sys::fuzz_target;
use replay::container::Replay;

fn read(replay: Replay) {
    let source = Source::from_replay(
        SourceKey {
            source_id: "fuzz".into(),
            revision: "fuzz".into(),
        },
        PathBuf::from("synthetic.battleplan"),
        Arc::new(replay),
        &AtomicBool::new(false),
    );
    if let Ok(source) = source {
        let _ = source.metadata();
    }
}

fuzz_target!(|data: &[u8]| {
    if let Ok(replay) = Replay::open(data.to_vec()) {
        read(replay);
    }
    if let Ok(replay) = battleplan_fuzz::raw_recording(data) {
        read(replay);
    }
});
