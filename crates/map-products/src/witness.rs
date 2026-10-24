use content_witness::{Recorder, Witness};
use installed_content_core::{
    ResolvedLayers,
    content::{ContentRead, ReadOutcome},
};
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
    pub fn session_note(&self, display: &str) {
        self.recorder.record_note("session-note", display);
    }
    pub fn session_error(&self, error: &io::Error) {
        self.recorder.record_error("session-error", error);
    }
    pub fn witness(&self) -> io::Result<Witness> {
        self.recorder.snapshot()
    }
    #[cfg(test)]
    pub(crate) fn entries(&self) -> Vec<content_witness::Entry> {
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
