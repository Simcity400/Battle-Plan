#![no_main]

use libfuzzer_sys::fuzz_target;
use replay::campaign::Replay;

fuzz_target!(|data: &[u8]| {
    let _ = Replay::open(data.to_vec());
});
