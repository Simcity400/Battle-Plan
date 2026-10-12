#![no_main]

use libfuzzer_sys::fuzz_target;
use replay::container::Replay;

fuzz_target!(|data: &[u8]| {
    let _ = Replay::read_partial_from(&mut std::io::Cursor::new(data), data.len() as u64);
});
