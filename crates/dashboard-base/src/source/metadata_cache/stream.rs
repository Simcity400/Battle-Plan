use serde::Serialize;
use std::{
    io::{self, Read, Write},
    sync::mpsc,
};

const CHUNK_BYTES: usize = 8192;
struct Chunk {
    bytes: [u8; CHUNK_BYTES],
    len: usize,
}
impl Default for Chunk {
    fn default() -> Self {
        Self {
            bytes: [0; CHUNK_BYTES],
            len: 0,
        }
    }
}
struct Encoder {
    sender: mpsc::SyncSender<Chunk>,
    pending: Chunk,
}
impl Write for Encoder {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let count = bytes.len().min(CHUNK_BYTES - self.pending.len);
        let end = self.pending.len + count;
        self.pending.bytes[self.pending.len..end].copy_from_slice(&bytes[..count]);
        self.pending.len = end;
        if end == CHUNK_BYTES {
            self.flush()?;
        }
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        if self.pending.len != 0 {
            self.sender
                .send(std::mem::take(&mut self.pending))
                .map_err(|_| {
                    io::Error::new(io::ErrorKind::BrokenPipe, "Metadata cache reader closed")
                })?;
        }
        Ok(())
    }
}
struct Decoder {
    receiver: mpsc::Receiver<Chunk>,
    current: Chunk,
    at: usize,
}
impl Read for Decoder {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        if self.at == self.current.len {
            match self.receiver.recv() {
                Ok(chunk) => {
                    self.current = chunk;
                    self.at = 0;
                }
                Err(_) => return Ok(0),
            }
        }
        let count = bytes.len().min(self.current.len - self.at);
        bytes[..count].copy_from_slice(&self.current.bytes[self.at..self.at + count]);
        self.at += count;
        Ok(count)
    }
}

pub(super) fn write<T: Serialize + Sync>(
    writer: &mut store::Writer,
    section: &str,
    value: &T,
) -> io::Result<()> {
    encode(value, |reader| writer.write_at(section, 0, reader))
}

fn encode<T: Serialize + Sync>(
    value: &T,
    consume: impl FnOnce(Decoder) -> io::Result<()>,
) -> io::Result<()> {
    let (sender, receiver) = mpsc::sync_channel(1);
    std::thread::scope(|scope| {
        let cpu = replay_frame::cpu::Context::capture();
        let encoder = std::thread::Builder::new()
            .name("metadata cache JSON".into())
            .spawn_scoped(scope, move || -> io::Result<()> {
                let _cpu = cpu.enter("metadata cache JSON");
                let mut output = Encoder {
                    sender,
                    pending: Chunk::default(),
                };
                serde_json::to_writer(&mut output, value)?;
                output.flush()
            })?;
        let written = consume(Decoder {
            receiver,
            current: Chunk::default(),
            at: 0,
        });
        let encoded = encoder
            .join()
            .map_err(|_| io::Error::other("Metadata cache serializer panicked"))?;
        written.and(encoded)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reader_failure_releases_a_blocked_encoder() {
        let large = "x".repeat(CHUNK_BYTES * 8);
        let error = encode(&large, |_reader| {
            Err(io::Error::other("synthetic store failure"))
        })
        .unwrap_err();
        assert_eq!(error.to_string(), "synthetic store failure");
    }

    #[test]
    fn encoding_failure_is_not_published_as_successful_truncated_json() {
        struct Fails;
        impl Serialize for Fails {
            fn serialize<S: serde::Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
                Err(serde::ser::Error::custom("synthetic serializer failure"))
            }
        }
        let error = encode(&Fails, |mut reader| {
            io::copy(&mut reader, &mut io::sink()).map(|_| ())
        })
        .unwrap_err();
        assert!(error.to_string().contains("synthetic serializer failure"));
    }
}
