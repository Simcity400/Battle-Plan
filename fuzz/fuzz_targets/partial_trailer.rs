#![no_main]

use std::io::Write;

use libfuzzer_sys::fuzz_target;
use replay::campaign::Summary;

fuzz_target!(|data: &[u8]| {
    let mut file = tempfile::tempfile().expect("create a temporary recording");
    file.write_all(data).expect("write the temporary recording");
    let _ = Summary::read_from(file);
});
