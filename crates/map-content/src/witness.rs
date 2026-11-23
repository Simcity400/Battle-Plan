use content_witness::{Recorder, Witness};
use content_reader_layers::{ResolvedLayers, content::{ContentRead, ReadOutcome}};
use sha2::{Digest, Sha256};
use std::{io, sync::Arc};

#[derive(Clone)]
pub struct ContentReads {
    layers: ResolvedLayers,
    recorder: Recorder,
}
impl ContentReads {
    pub fn new(layers: &ResolvedLayers) -> Self {
        let recorder = Recorder::new(layers.presentation());
        let log = recorder.for_layers(layers);
        Self {
            layers: layers
                .with_read_trace(Arc::new(move |read| log.record_read(read)))
                .with_read_purpose("content"),
            recorder,
        }
    }
    pub fn reader(&self, layers: &ResolvedLayers, purpose: &str) -> ResolvedLayers {
        let log = self.recorder.for_layers(layers);
        layers
            .with_read_trace(Arc::new(move |read| log.record_read(read)))
            .with_read_purpose(purpose)
    }
    pub fn load<T>(&self, load: impl FnOnce(&ResolvedLayers) -> io::Result<T>) -> io::Result<T> {
        let result = load(&self.layers);
        if let Err(error) = &result {
            self.session_error(error);
        }
        result
    }
    pub fn load_retryable<T>(
        &self,
        load: impl FnOnce(&ResolvedLayers) -> io::Result<T>,
    ) -> io::Result<T> {
        let result = load(&self.layers);
        if let Err(error) = &result {
            if !matches!(
                error.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::OutOfMemory
            ) {
                self.session_error(error);
            }
        }
        result
    }
    pub fn user_bytes(&self, path: &[u8], result: &io::Result<Vec<u8>>) {
        let outcome = match result {
            Ok(bytes) => ReadOutcome::Bytes {
                length: bytes.len() as u64,
                digest: Sha256::digest(bytes).into(),
            },
            Err(error) if error.kind() == io::ErrorKind::NotFound => ReadOutcome::Probe(false),
            Err(error) => ReadOutcome::Error {
                kind: format!("{:?}", error.kind()),
                message: error.to_string(),
                payload: None,
            },
        };
        self.recorder.record_read(ContentRead {
            purpose: "audio/user-settings".into(),
            path: path.to_vec(),
            layer: None,
            outcome,
        });
    }
    pub fn operand_bytes(&self, name: &[u8], bytes: &[u8]) {
        self.operand_digest(name, bytes.len() as u64, Sha256::digest(bytes).into());
    }
    pub fn operand_stream(
        &self,
        name: &[u8],
        write: impl FnOnce(&mut dyn io::Write) -> io::Result<()>,
    ) -> io::Result<()> {
        let mut buffer = Vec::new();
        buffer
            .try_reserve_exact(OPERAND_BUFFER_BYTES)
            .map_err(|_| io::Error::from(io::ErrorKind::OutOfMemory))?;
        let mut output = OperandHash {
            hash: Sha256::new(),
            length: 0,
            buffer,
        };
        write(&mut output)?;
        io::Write::flush(&mut output)?;
        self.operand_digest(name, output.length, output.hash.finalize().into());
        Ok(())
    }
    fn operand_digest(&self, name: &[u8], length: u64, digest: [u8; 32]) {
        self.recorder.record_read(ContentRead {
            purpose: "display/owned-content".into(),
            path: name.to_vec(),
            layer: None,
            outcome: ReadOutcome::Bytes { length, digest },
        });
    }
    pub fn session_note(&self, display: &str) {
        self.recorder.record_note("session-note", display);
    }
    pub fn session_error(&self, error: &io::Error) {
        self.recorder.record_error("session-error", error);
    }
    pub fn binding(&self, purposes: &[&str]) -> io::Result<[u8; 32]> {
        Ok(self.recorder.snapshot_for(purposes)?.digest)
    }
    pub fn dependent_binding(&self, dependent: &str, purposes: &[&str]) -> io::Result<[u8; 32]> {
        let content = self.binding(purposes)?;
        let mut digest = Sha256::new();
        digest.update(b"display/dependent-content/v1");
        digest.update((dependent.len() as u64).to_le_bytes());
        digest.update(dependent.as_bytes());
        digest.update(content);
        Ok(digest.finalize().into())
    }
    pub fn witness(&self) -> io::Result<Witness> {
        self.recorder.snapshot()
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn entries(&self) -> Vec<content_witness::Entry> {
        let mut entries = Vec::new();
        self.witness()
            .unwrap()
            .visit(|_, bytes| {
                entries.push(product_codec::decode(bytes)?);
                Ok(())
            })
            .unwrap();
        entries
    }
}

const OPERAND_BUFFER_BYTES: usize = 64 * 1024;
struct OperandHash {
    hash: Sha256,
    length: u64,
    buffer: Vec<u8>,
}
impl io::Write for OperandHash {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.length = self
            .length
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| io::Error::other("display content operand length overflow"))?;
        let length = bytes.len();
        let mut remaining = bytes;
        while !remaining.is_empty() {
            if self.buffer.is_empty() && remaining.len() >= OPERAND_BUFFER_BYTES {
                self.hash.update(remaining);
                break;
            }
            let take = remaining
                .len()
                .min(OPERAND_BUFFER_BYTES - self.buffer.len());
            self.buffer.extend_from_slice(&remaining[..take]);
            remaining = &remaining[take..];
            if self.buffer.len() == OPERAND_BUFFER_BYTES {
                io::Write::flush(self)?;
            }
        }
        Ok(length)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.hash.update(&self.buffer);
        self.buffer.clear();
        Ok(())
    }
}
