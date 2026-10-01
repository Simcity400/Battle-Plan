use dashboard::protocol::MAX_MESSAGE_BYTES;
use std::io::{BufRead, BufReader, Read};

/// Bound even a malformed diagnostic line; discard its remainder in chunks.
pub(super) fn read_lines(input: impl Read, mut visit: impl FnMut(&str)) {
    let mut reader = BufReader::new(input);
    let mut bytes = Vec::new();
    let mut discarding = false;
    loop {
        bytes.clear();
        let count = match reader
            .by_ref()
            .take((MAX_MESSAGE_BYTES + 1) as u64)
            .read_until(b'\n', &mut bytes)
        {
            Ok(0) | Err(_) => break,
            Ok(count) => count,
        };
        let complete = bytes.last() == Some(&b'\n');
        if !discarding
            && count <= MAX_MESSAGE_BYTES
            && complete
            && let Ok(line) = std::str::from_utf8(&bytes)
        {
            visit(line.trim_end());
        }
        discarding = !complete;
    }
}
