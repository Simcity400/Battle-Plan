use std::{collections::HashMap, io};
use super::{dated, delta};

const PATCHED: &[u8; 4] = b"PND1";
const DELTA: u8 = 8;
const HEADER: usize = 24;
const MAX_BASE_BYTES: usize = crate::lead::BYTES / 2;

#[derive(Default)]
pub struct PanelBodies {
    bases: HashMap<u32, Base>,
    bytes: usize,
}

struct Base {
    stamp: (u32, u32),
    bytes: Box<[u8]>,
}

impl PanelBodies {
    fn forget(&mut self, id: u32) {
        if let Some(old) = self.bases.remove(&id) {
            self.bytes -= old.bytes.len();
        }
    }

    fn remember(&mut self, id: u32, stamp: (u32, u32), body: &[u8]) {
        self.forget(id);
        if body.len() > MAX_BASE_BYTES { return; }
        if self.bytes + body.len() > MAX_BASE_BYTES { *self = Self::default(); }
        self.bytes += body.len();
        self.bases.insert(id, Base { stamp, bytes: body.into() });
    }

    fn base(&self, id: u32, stamp: (u32, u32)) -> Option<&[u8]> {
        self.bases.get(&id).filter(|base| base.stamp == stamp).map(|base| &*base.bytes)
    }

    pub fn encode(&mut self, bytes: &[u8]) -> Option<Vec<u8>> {
        let stamp = dated::stamp(bytes)?;
        let mut output = None;
        let mut at = HEADER;
        for record in dated::records(bytes).ok()? {
            let end = at + record.byte_len();
            match record.kind {
                dated::DATA => {
                    let patch = self.base(record.id, stamp).and_then(|base| delta::encode(base, record.body));
                    self.remember(record.id, stamp, record.body);
                    if let Some(patch) = patch {
                        let output = output.get_or_insert_with(|| {
                            let mut prefix = bytes[..at].to_vec();
                            prefix[..4].copy_from_slice(PATCHED);
                            prefix
                        });
                        append(output, &bytes[at..at + 16], DELTA, &patch);
                        at = end;
                        continue;
                    }
                }
                dated::REFUSED | dated::STOPPED => self.forget(record.id),
                _ => {}
            }
            if let Some(output) = &mut output { output.extend_from_slice(&bytes[at..end]); }
            at = end;
        }
        output
    }

    pub fn decode(&mut self, bytes: &[u8]) -> io::Result<Option<Vec<u8>>> {
        let encoded = bytes.starts_with(PATCHED);
        let canonical = encoded.then(|| {
            let mut packet = bytes.to_vec();
            packet[..4].copy_from_slice(dated::MAGIC);
            packet
        });
        let bytes = canonical.as_deref().unwrap_or(bytes);
        let Some(stamp) = dated::stamp(bytes) else {
            if encoded { return Err(mismatch()); }
            return Ok(None);
        };
        let mut output = encoded.then(|| bytes[..HEADER].to_vec());
        let mut at = HEADER;
        for record in dated::records(bytes)? {
            let end = at + record.byte_len();
            match record.kind {
                DELTA if encoded => {
                    let base = self.base(record.id, stamp).ok_or_else(mismatch)?;
                    let value = delta::apply(base, record.body).ok_or_else(mismatch)?;
                    self.remember(record.id, stamp, &value);
                    append(output.as_mut().expect("encoded packet"), &bytes[at..at + 16], dated::DATA, &value);
                    at = end;
                    continue;
                }
                dated::DATA => self.remember(record.id, stamp, record.body),
                dated::REFUSED | dated::STOPPED => self.forget(record.id),
                _ => {}
            }
            if let Some(output) = &mut output { output.extend_from_slice(&bytes[at..end]); }
            at = end;
        }
        Ok(output)
    }
}

fn append(output: &mut Vec<u8>, header: &[u8], kind: u8, body: &[u8]) {
    let at = output.len();
    output.extend_from_slice(header);
    output[at + 8] = kind;
    output[at + 12..at + 16].copy_from_slice(&(body.len() as u32).to_le_bytes());
    output.extend_from_slice(body);
    output.resize(output.len().next_multiple_of(8), 0);
}

fn mismatch() -> io::Error { crate::invalid("Panel delta does not match its base or result") }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decoding_restores_the_whole_batch_after_a_missing_base() {
        let seed = dated::encoded(0, 1, 2, 101, 20, dated::DATA, &[b'x'; 256]);
        let mut batch = dated::ProductBatch::new((0, 1, 2));
        batch.changed(103, 21, dated::DATA, b"other reading");
        batch.changed(101, 21, dated::DATA, &[&[b'y'], &[b'x'; 255][..]].concat());
        let next = batch.finish(Some(21));
        let mut encoder = PanelBodies::default();
        assert!(encoder.encode(&seed).is_none());
        let encoded = encoder.encode(&next).expect("unchanged span");
        let mut decoder = PanelBodies::default();
        assert!(decoder.decode(&encoded).is_err());
        assert!(decoder.decode(&seed).unwrap().is_none());
        assert_eq!(decoder.decode(&encoded).unwrap(), Some(next.clone()));
        assert!(decoder.decode(&next).unwrap().is_none(), "a full repair can seed the next delta");
        let seek = dated::encoded(0, 1, 3, 101, 21, dated::DATA, &[b'x'; 256]);
        assert!(encoder.encode(&seek).is_none());
    }
}
