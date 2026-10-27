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
    pub fn operand_bytes(&self, name: &[u8], bytes: &[u8]) {
        self.operand_digest(name, bytes.len() as u64, Sha256::digest(bytes).into());
    }
    pub fn operand_stream(
        &self,
        name: &[u8],
        write: impl FnOnce(&mut dyn io::Write) -> io::Result<()>,
    ) -> io::Result<()> {
        use std::io::Write;
        struct DigestWriter {
            hash: Sha256,
            length: u64,
        }
        impl io::Write for DigestWriter {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                self.length = self.length.checked_add(bytes.len() as u64)
                    .ok_or_else(|| io::Error::from(io::ErrorKind::OutOfMemory))?;
                self.hash.update(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> io::Result<()> { Ok(()) }
        }
        const BUFFER: usize = 8192;
        let _credit = map_display_data::credit::display_products().admit_item(BUFFER)?;
        let mut digest = DigestWriter { hash: Sha256::new(), length: 0 };
        {
            let mut buffered = io::BufWriter::with_capacity(BUFFER, &mut digest);
            write(&mut buffered)?;
            buffered.flush()?;
        }
        self.operand_digest(name, digest.length, digest.hash.finalize().into());
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

// Weak bank keys keep allocation identities distinct without retaining immutable payloads.
pub(crate) struct Operand<T>(std::sync::Weak<T>);
impl<T> Operand<T> {
    pub fn new(value: &Arc<T>) -> Self {
        Self(Arc::downgrade(value))
    }
}
impl<T> Clone for Operand<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}
impl<T> PartialEq for Operand<T> {
    fn eq(&self, other: &Self) -> bool {
        self.0.ptr_eq(&other.0)
    }
}
impl<T> Eq for Operand<T> {}
impl<T> PartialOrd for Operand<T> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl<T> Ord for Operand<T> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.as_ptr().cmp(&other.0.as_ptr())
    }
}

#[derive(Clone)]
pub(crate) struct KeyText(Arc<ChargedText>);
struct ChargedText {
    value: String,
    _credit: map_display_data::credit::Credit,
}
impl KeyText {
    pub fn new(value: &str) -> io::Result<Self> {
        let bytes = value
            .len()
            .checked_add(
                size_of::<ChargedText>() + 2 * size_of::<usize>() + align_of::<ChargedText>(),
            )
            .ok_or_else(|| io::Error::from(io::ErrorKind::OutOfMemory))?;
        let credit = map_display_data::credit::display_products().admit(bytes)?;
        Ok(Self(Arc::new(ChargedText {
            value: value.to_owned(),
            _credit: credit,
        })))
    }
}
impl PartialEq for KeyText {
    fn eq(&self, other: &Self) -> bool {
        self.0.value == other.0.value
    }
}
impl Eq for KeyText {}
impl PartialOrd for KeyText {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for KeyText {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.value.cmp(&other.0.value)
    }
}

#[cfg(test)]
mod operand_tests {
    use super::*;
    #[test]
    fn weak_input_keys_prevent_mutable_alias_and_address_reuse_without_pinning_payloads() {
        let mut bank = Arc::new(vec![1u8, 2]);
        let old = Operand::new(&bank);
        assert!(Arc::get_mut(&mut bank).is_none());
        assert!(old == old.clone());
        let payload = Arc::downgrade(&bank);
        drop(bank);
        assert!(payload.upgrade().is_none());
        let replacement = Arc::new(vec![1u8, 2]);
        assert!(old != Operand::new(&replacement));
    }
}
